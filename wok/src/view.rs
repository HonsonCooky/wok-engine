//! The chrome composition root: the one place the editor's regions are ordered, and the single entry
//! both the live frame loop (`crate::main`) and the snapshot test render through. Sharing this seam is
//! what makes the snapshot a real regression guard - the PNG shows exactly what the app draws, because
//! both go through `chrome`.
//!
//! The regions read the [`Model`](crate::model::Model) and emit [`Action`](crate::action::Action)s;
//! they never mutate the model themselves. `chrome` collects the frame's actions into a buffer and
//! returns it, and the caller applies each through `crate::action::handle` - the single writer. The
//! snapshot test renders the same `chrome` over a model it builds directly, so the PNG tracks exactly
//! what the app draws.
//!
//! Region order is layout order and load-bearing (see the README shell layout, sharp-edges 2): the
//! navigation panel is shown first so it claims its full-height strip, then the view column - the
//! status bar at the bottom, the tab bar at the top, the editor well filling the rest - spans only the
//! remaining width, so the status bar never runs under the nav. This holds whichever side the panel
//! docks to: it is added before the view column on either side. When the panel is hidden the view
//! column spans the full width. The status bar reads the open project (its name, or that none is open);
//! the tab bar renders the open tabs and the editor well names the active one (an empty well when none
//! is open); the nav panel, the status bar, the tab bar, and the hamburger's menus read the model and
//! emit actions.
//!
//! Above the well sits the floating layer: the conditional inspector (`crate::inspector`), an
//! `egui::Window` clipped to the editor area and present only when a selection resolves to a placement.
//! It is shown after the well so it layers over it, and Esc (when something is selected) emits a
//! deselect - both read the same `model.shell.selection()` the Instances tree sets.

use crate::action::Action;
use crate::inspector;
use crate::loaded::LoadedScene;
use crate::menu;
use crate::model::Model;
use crate::workspace;

/// Render the full editor chrome for one frame: the navigation panel first (full height on its docked
/// side, and only when visible), then the view column's status bar, tab bar, and editor well, and last
/// the floating inspector over the well. Returns the actions the regions emitted this frame, for the
/// caller to apply through `crate::action::handle`.
///
/// `loaded_scene` is the active scene tab's loaded data (reconciled by the frame loop, `crate::loaded`),
/// which the Instances nav view lists; it is `None` when no scene tab is active. The model alone cannot
/// carry it - it is filesystem residency, not pure model state - so it is threaded in separately.
pub fn chrome(ctx: &egui::Context, model: &Model, loaded_scene: Option<&LoadedScene>) -> Vec<Action> {
    let mut actions = Vec::new();
    // Region order is load-bearing (sharp-edges 2): the nav panel is added first on whichever side it
    // docks, so it claims its full-height strip and the view column fills the rest - the status bar
    // never runs under the nav. Hidden, the view column spans the full width.
    if model.shell.nav_visible() {
        workspace::nav_panel(ctx, model, loaded_scene, &mut actions);
    }
    menu::status_bar(ctx, model.project.as_ref());
    workspace::tab_bar(ctx, model, &mut actions);
    // The editor area is the central region left after the three bounding panels; capture it now, before
    // the central panel consumes it, so the floating inspector can anchor to and clip to it. The
    // inspector is shown after the well so it layers over it; it appears only when a selection resolves.
    let editor_rect = ctx.available_rect();
    workspace::editor_area(ctx, model, &mut actions);
    inspector::floating(ctx, model, loaded_scene, editor_rect);
    // Esc clears the selection (editor-design.md: Esc unwinds the selection). Gated on there being one,
    // so it is inert otherwise and never fights for the key when nothing is selected.
    if model.shell.selection().is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(Action::Deselect);
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InstanceSort, NavSide, NavView, Tab};
    use crate::project::Project;
    use crate::recent::Recents;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use glam::{Quat, Vec3};
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard};
    use wok_scene::{
        Chunk, ChunkCoord, ChunkStreaming, ContentLayout, Eagerness, InstanceId, LightStateRef, Placement,
        PrefabRef, Scene, StreamingDefaults, Transform, save_chunk, save_scene,
    };

    /// Serializes the wgpu snapshot tests. egui_kittest builds a fresh headless wgpu device per
    /// harness, and creating or tearing several down concurrently crashes on some Windows drivers - a
    /// wgpu teardown race, not a fault in the chrome. Each GPU test holds this lock for its lifetime,
    /// so only one device is ever alive at a time. Poison is ignored, so a failed snapshot assert does
    /// not cascade into the rest.
    static GPU_TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Acquire the GPU-test lock, recovering from a poisoned mutex (a prior test's panic) so the lock
    /// still serializes rather than failing every later test.
    fn gpu_guard() -> MutexGuard<'static, ()> {
        GPU_TEST_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A model with `active` as the navigation view, built directly (not by faking a click) so each
    /// snapshot pins the chrome in a specific state - the icon-bar accent, the header label, and the
    /// placeholder body all track this view (sharp-edges 3: construct states by building the model).
    fn model_with(active: NavView) -> Model {
        let mut model = Model::default();
        model.shell.select_nav(active);
        model
    }

    /// A model with `scenes` opened as tabs (in order) and the tab at `active` focused, built directly
    /// through the tab mutators (not by faking clicks) so the snapshot pins a specific tab-bar state -
    /// the active-tab styling and the editor-well placeholder both track it (sharp-edges 3). The
    /// navigation view is left at the default; the tabs exercise the tab chrome independently of any
    /// open project or nav content.
    fn model_with_tabs(scenes: &[&str], active: usize) -> Model {
        let mut model = Model::default();
        for scene in scenes {
            model.shell.open_tab(Tab::Scene((*scene).to_string()));
        }
        model.shell.select_tab(active);
        model
    }

    /// Build a harness that renders the chrome at `size` under `theme` over `model` and `loaded_scene`
    /// (the active scene tab's loaded data, as the live frame loop threads in - `None` for the states
    /// that have no scene loaded), with the editor surface filled behind the transparent editor well
    /// (standing in for the in-app GPU clear), so the snapshot reads as it does live in that theme.
    /// Forcing the theme keeps each snapshot deterministic regardless of the host's OS setting. The
    /// collected actions are discarded - a static render emits none.
    fn chrome_harness(
        theme: egui::ThemePreference,
        size: egui::Vec2,
        model: Model,
        loaded_scene: Option<LoadedScene>,
    ) -> Harness<'static> {
        Harness::builder().with_size(size).wgpu().build(move |ctx| {
            crate::theme::apply(ctx);
            ctx.set_theme(theme);
            let editor_bg = crate::theme::palette(ctx).editor_bg;
            ctx.layer_painter(egui::LayerId::background()).rect_filled(ctx.screen_rect(), 0.0, editor_bg);
            let _ = chrome(ctx, &model, loaded_scene.as_ref());
        })
    }

    /// Render `model`'s chrome to a PNG under `tests/snapshots`, through the composition root the app
    /// renders through, so the PNG tracks the real chrome. Refresh after an intended look change with
    /// `UPDATE_SNAPSHOTS=1 cargo test -p wok` and commit the new PNG.
    fn snapshot_of(name: &str, theme: egui::ThemePreference, model: Model) {
        let _gpu = gpu_guard();
        // The states routed through here have no scene loaded (the nav-view, tab, dock, and menu
        // snapshots); the Instances-with-placements states seed their own scene and build the harness
        // directly (the `chrome_instances_*` tests).
        let mut harness = chrome_harness(theme, egui::vec2(1100.0, 700.0), model, None);
        harness.run();
        harness.snapshot(name);
    }

    /// Snapshot the chrome with `active` as the navigation view - the common case where only the active
    /// view varies.
    fn snapshot(name: &str, theme: egui::ThemePreference, active: NavView) {
        snapshot_of(name, theme, model_with(active));
    }

    /// The Instances view active with no scene loaded, in dark and light - the light/dark palette
    /// guard. The body shows the Instances "No scene open" empty state (no scene tab is active, so the
    /// frame loop loads nothing). Scenes, not Instances, is the default landing view (since
    /// 2026-06-23); that default state is the `chrome_scenes` pair below, and the Instances view
    /// listing a loaded scene's placements is the `chrome_instances_*` trio (grouped, collapsed, flat).
    #[test]
    fn chrome_snapshot() {
        snapshot("chrome", egui::ThemePreference::Dark, NavView::Instances);
    }

    #[test]
    fn chrome_light_snapshot() {
        snapshot("chrome_light", egui::ThemePreference::Light, NavView::Instances);
    }

    /// The default landing state - the Scenes view active (the project group's first) with no project
    /// open, in dark and light: the body shows the Scenes view's "No project open" empty state (the
    /// default model has no project). The pair guards that the chrome tracks `model.shell.active_nav`
    /// (compare the `chrome` pair, which pins Instances) and that the no-project empty state reads
    /// cleanly in each theme.
    #[test]
    fn chrome_scenes_snapshot() {
        snapshot("chrome_scenes", egui::ThemePreference::Dark, NavView::Scenes);
    }

    #[test]
    fn chrome_scenes_light_snapshot() {
        snapshot("chrome_scenes_light", egui::ThemePreference::Light, NavView::Scenes);
    }

    /// The Scenes view over an open project whose `assets/scenes` holds two scene folders: the body
    /// lists their names, sorted ("alpha" above "beta"), one row each, in place of the empty state.
    /// Seeds a temp project on disk and lets the chrome scan it per frame, exactly as the live app
    /// does (`workspace::content_list`), rather than injecting a name list - so this guards the
    /// `ContentLayout` wiring end to end. The root's leaf is the fixed "DemoGame", so both the listed
    /// rows and the status bar's project name are deterministic (a pid/temp suffix would not be). Dark
    /// alone: this is a content state, not a palette one (the pair above guards the themes).
    #[test]
    fn chrome_scenes_listed_snapshot() {
        let _gpu = gpu_guard();
        // A fixed-leaf root under a distinctive parent: the leaf names the project in the status bar,
        // the parent keeps this tree clear of other tests'. Cleared first so a crashed run cannot leave
        // stray scenes that would change the listing.
        let parent = std::env::temp_dir().join("wok-nav-listed-snapshot");
        let root = parent.join("DemoGame");
        let _ = std::fs::remove_dir_all(&parent);
        for scene in ["alpha", "beta"] {
            std::fs::create_dir_all(root.join("assets").join("scenes").join(scene)).unwrap();
        }

        let mut model = model_with(NavView::Scenes);
        model.project = Some(Project::new(root.clone()));
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, None);
        harness.run();
        harness.snapshot("chrome_scenes_listed");

        let _ = std::fs::remove_dir_all(&parent);
    }

    /// The Prefabs view over an open project whose `assets/prefabs` holds two prefab files: the body
    /// lists their slugs, sorted ("barrel" above "oak_tree"), one display-only row each led by the
    /// filled-cube type glyph (`icons::CUBE`) - the project-scoped rows now carry a leading type glyph
    /// so they read as one set with the Instances tree. Seeds a temp project on disk and lets the chrome
    /// scan it per frame, exactly as the live app does, so this also guards the `prefab_slugs` wiring.
    /// Same fixed-leaf "DemoGame" root as the Scenes listing for a deterministic project name. Dark
    /// alone: a content state, not a palette one.
    #[test]
    fn chrome_prefabs_listed_snapshot() {
        let _gpu = gpu_guard();
        let parent = std::env::temp_dir().join("wok-nav-prefabs-snapshot");
        let root = parent.join("DemoGame");
        let _ = std::fs::remove_dir_all(&parent);
        let layout = ContentLayout::new(&root);
        std::fs::create_dir_all(layout.prefabs_dir()).unwrap();
        for slug in ["barrel", "oak_tree"] {
            std::fs::write(layout.prefab(slug), b"{}").unwrap();
        }

        let mut model = model_with(NavView::Prefabs);
        model.project = Some(Project::new(root.clone()));
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, None);
        harness.run();
        harness.snapshot("chrome_prefabs_listed");

        let _ = std::fs::remove_dir_all(&parent);
    }

    /// The Lighting view over an open project whose `assets/lighting` holds two light-state files: the
    /// body lists their names, sorted ("dawn" above "noon"), one display-only row each led by the sun
    /// type glyph (`icons::WEATHER_SUNNY`). Seeds a temp project on disk and scans it per frame as the
    /// live app does, guarding the `lighting_names` wiring end to end. Same fixed-leaf "DemoGame" root.
    /// Dark alone: a content state, not a palette one.
    #[test]
    fn chrome_lighting_listed_snapshot() {
        let _gpu = gpu_guard();
        let parent = std::env::temp_dir().join("wok-nav-lighting-snapshot");
        let root = parent.join("DemoGame");
        let _ = std::fs::remove_dir_all(&parent);
        let layout = ContentLayout::new(&root);
        std::fs::create_dir_all(layout.lighting_dir()).unwrap();
        for name in ["dawn", "noon"] {
            std::fs::write(layout.lighting(name), b"{}").unwrap();
        }

        let mut model = model_with(NavView::Lighting);
        model.project = Some(Project::new(root.clone()));
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, None);
        harness.run();
        harness.snapshot("chrome_lighting_listed");

        let _ = std::fs::remove_dir_all(&parent);
    }

    /// A well-formed scene on disk spanning two prefabs with several instances each, so the grouped
    /// tree shows real groups and counts: three `oak_tree`s (one named "the landmark oak", two on the
    /// `{prefab} #{id}` fallback) and two `rock`s. Written as one chunk in scrambled order under
    /// `root`/`scene`; the residency sorts by instance id (0..=4), so the listed order is deterministic
    /// regardless of the file order. The fixed ids also fix both orderings under test - grouped (by
    /// prefab name, then id) and flat (A-Z by row label, where "the landmark oak" sorts after the
    /// `rock` fallbacks rather than at its id-1 slot).
    ///
    /// "the landmark oak" (id 1) gets a non-identity transform - a translation, a 45 degree yaw, and a
    /// 1.5 uniform scale - so the inspector snapshot (which selects it) shows real Pos / Rot / Scale
    /// values, the yaw landing in the Y field (the YXZ Euler readout). It is invisible to the other
    /// Instances snapshots, which show row labels only, never the transform.
    fn seed_instances_scene(root: &std::path::Path, scene: &str) {
        let layout = ContentLayout::new(root);
        std::fs::create_dir_all(layout.scene_dir(scene)).unwrap();
        let manifest = Scene {
            name: scene.to_string(),
            default_lighting: LightStateRef::new("noon"),
            regions: vec![],
            default_streaming: StreamingDefaults { load_radius: 3, default_eagerness: Eagerness::Eager },
            next_instance_id: InstanceId(5),
        };
        save_scene(&manifest, layout.scene_json(scene)).unwrap();
        let placement = |prefab: &str, id: u32, name: Option<&str>| Placement {
            prefab: PrefabRef::new(prefab),
            instance_id: InstanceId(id),
            name: name.map(str::to_owned),
            transform: Transform::IDENTITY,
            state: None,
        };
        let landmark = Placement {
            prefab: PrefabRef::new("oak_tree"),
            instance_id: InstanceId(1),
            name: Some("the landmark oak".to_owned()),
            transform: Transform {
                translation: Vec3::new(12.0, 0.5, -3.0),
                rotation: Quat::from_rotation_y(45.0_f32.to_radians()),
                scale: Vec3::splat(1.5),
            },
            state: None,
        };
        let chunk = Chunk {
            coord: ChunkCoord::new(0, 0),
            placements: vec![
                placement("rock", 3, None),
                placement("oak_tree", 4, None),
                placement("oak_tree", 0, None),
                placement("rock", 2, None),
                landmark,
            ],
            streaming: ChunkStreaming::default(),
        };
        save_chunk(&chunk, layout.chunk(scene, ChunkCoord::new(0, 0))).unwrap();
    }

    /// Seed the two-prefab scene under a fixed-leaf "DemoGame" root (so the status-bar project name is
    /// deterministic), and build the model with that project open, the scene opened as the active tab,
    /// and the Instances view selected - the state a Scenes-row click then a switch to Instances
    /// produces. Returns the model, the scene loaded through the real residency path
    /// (`LoadedScene::load`, so the snapshots guard load-and-list end to end), and the temp parent to
    /// remove when the test is done. `leaf` keeps each Instances snapshot's tree clear of the others'.
    fn seed_instances_model(leaf: &str) -> (Model, LoadedScene, PathBuf) {
        let parent = std::env::temp_dir().join(leaf);
        let root = parent.join("DemoGame");
        let _ = std::fs::remove_dir_all(&parent);
        seed_instances_scene(&root, "village");

        let mut model = model_with(NavView::Instances);
        model.project = Some(Project::new(root.clone()));
        model.shell.open_tab(Tab::Scene("village".to_string()));
        let loaded = LoadedScene::load(&root, "village");
        (model, loaded, parent)
    }

    /// The Instances view's grouped tree (the default sort): two group rows, sorted by prefab name -
    /// "oak_tree" (count 3) above "rock" (count 2) - each open (an open-folder glyph, no chevron), with
    /// their instance rows indented under them in id order. The named placement reads "the landmark oak"
    /// mid-group; the rest show the `{prefab} #{id}` fallback. Nothing selected, so no row is highlighted
    /// and no inspector shows - this also serves as the deselected / no-inspector state. Dark alone: this
    /// is a content state, not a palette one (the `chrome` pair guards themes).
    #[test]
    fn chrome_instances_grouped_snapshot() {
        let _gpu = gpu_guard();
        let (model, loaded, parent) = seed_instances_model("wok-instances-grouped-snapshot");
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, Some(loaded));
        harness.run();
        harness.snapshot("chrome_instances_grouped");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// The grouped tree with one group collapsed: the "oak_tree" group's open state is seeded `false`
    /// in egui memory - the same transient store the view reads, under the very id it computes
    /// (`workspace::instance_group_id`), since the open state is not model state a test could build
    /// directly. So "oak_tree" shows its header and count with its instance rows hidden (a right-facing
    /// chevron), while "rock" stays open below it. Dark alone: a layout state, not a palette one.
    #[test]
    fn chrome_instances_collapsed_snapshot() {
        let _gpu = gpu_guard();
        let (model, loaded, parent) = seed_instances_model("wok-instances-collapsed-snapshot");
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, Some(loaded));
        // Drive the "oak_tree" group collapsed by seeding the id the view reads, then render that frame.
        harness.ctx.data_mut(|d| d.insert_temp(crate::workspace::instance_group_id("village", "oak_tree"), false));
        harness.run();
        harness.snapshot("chrome_instances_collapsed");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// The Instances view's flat layout: every placement as one row, sorted A-Z by its display label.
    /// Built by setting the shell's sort mode to `Flat` directly (not by clicking the header toggle),
    /// so the snapshot pins the flat state (sharp-edges 3). The label order puts the two `oak_tree`
    /// fallbacks first, then the two `rock` fallbacks, then "the landmark oak" last - by name, not by
    /// its instance id. Dark alone: a content state, not a palette one.
    #[test]
    fn chrome_instances_flat_snapshot() {
        let _gpu = gpu_guard();
        let (mut model, loaded, parent) = seed_instances_model("wok-instances-flat-snapshot");
        model.shell.set_instance_sort(InstanceSort::Flat);
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, Some(loaded));
        harness.run();
        harness.snapshot("chrome_instances_flat");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// An instance selected from the tree: "the landmark oak" (id 1) is selected directly on the model,
    /// so its row in the grouped tree carries the full-bleed accent highlight and the floating inspector
    /// shows it - IDENTITY (the name, the `oak_tree` prefab, the `0x00000001` id) and TRANSFORM (its
    /// seeded Pos / 45-degree-yaw Rot / 1.5 Scale, the X/Y/Z fields axis-tinted), closing on the
    /// boundary line. Built by setting the selection directly (not by faking a row click), the
    /// counterpart to the grouped snapshot's deselected / no-inspector state (sharp-edges 3: build the
    /// state on the model). Dark alone: a content state, not a palette one.
    #[test]
    fn chrome_instances_selected_snapshot() {
        let _gpu = gpu_guard();
        let (mut model, loaded, parent) = seed_instances_model("wok-instances-selected-snapshot");
        model.shell.select(InstanceId(1));
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, Some(loaded));
        harness.run();
        harness.snapshot("chrome_instances_selected");
        let _ = std::fs::remove_dir_all(&parent);
    }

    /// The navigation panel hidden: the view column - tab bar, editor well, status bar - spans the full
    /// width with no nav strip. Dark alone is enough; this is a layout state, not a palette one (the
    /// themes are guarded by the pairs above).
    #[test]
    fn chrome_nav_hidden_snapshot() {
        let mut model = Model::default();
        model.shell.toggle_nav();
        snapshot_of("chrome_nav_hidden", egui::ThemePreference::Dark, model);
    }

    /// The navigation panel docked right: the nav strip on the right edge, the view column (with its
    /// status bar) confined to the remaining left width - the region-order rule holding on the right
    /// side, not just the left.
    #[test]
    fn chrome_nav_docked_right_snapshot() {
        let mut model = Model::default();
        model.shell.set_nav_side(NavSide::Right);
        snapshot_of("chrome_nav_docked_right", egui::ThemePreference::Dark, model);
    }

    /// One scene tab open and active: the tab bar shows the single tab styled active (the accent
    /// top-line and the editor-surface fill), and the editor well names the open scene in place of the
    /// empty well. Dark alone is enough; this is a content/layout state, not a palette one.
    #[test]
    fn chrome_one_tab_snapshot() {
        snapshot_of("chrome_one_tab", egui::ThemePreference::Dark, model_with_tabs(&["village"], 0));
    }

    /// Two scene tabs with the second active: the first sits inactive (flat and dim), the second active
    /// (the editor-surface fill and accent top-line), and the editor well names the active scene - the
    /// switching state the tab bar shows. Dark alone is enough; this is a layout state, not a palette one.
    #[test]
    fn chrome_two_tabs_snapshot() {
        snapshot_of("chrome_two_tabs", egui::ThemePreference::Dark, model_with_tabs(&["village", "dungeon"], 1));
    }

    /// The app-menu open at its View submenu, driven by clicking the hamburger then hovering View (the
    /// top button opens on an accesskit click, but egui opens a submenu on pointer hover - sharp-edges
    /// 3). Guards that the menu renders and the View items are present: Hide Navigation Panel (the label
    /// tracks the default visible state) and the Dock Left / Dock Right radios with Left marked.
    #[test]
    fn chrome_view_menu_snapshot() {
        let _gpu = gpu_guard();
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), Model::default(), None);
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        harness.get_by_label("View").hover();
        harness.run();
        harness.snapshot("chrome_view_menu");
    }

    /// A project open: the status bar's left shows the project's name (its folder leaf) rather than
    /// "No project open" - the in-window confirmation that Open took effect, mirroring the window
    /// title. Dark alone is enough; this is a content state, not a palette one.
    #[test]
    fn chrome_project_open_snapshot() {
        let model = Model { project: Some(Project::new("C:/games/MyGame")), ..Default::default() };
        snapshot_of("chrome_project_open", egui::ThemePreference::Dark, model);
    }

    /// The app-menu open at its File submenu, driven by clicking the hamburger then hovering File
    /// (egui opens a submenu on hover, not on an accesskit click - sharp-edges 3). Built over a model
    /// with a project open and a couple of recents, so every File item is live: Open Project..., Open
    /// Recent (enabled, with entries), and Close Project (enabled because a project is open).
    #[test]
    fn chrome_file_menu_snapshot() {
        let _gpu = gpu_guard();
        let model = Model {
            project: Some(Project::new("C:/games/MyGame")),
            recents: Recents::from_paths(["C:/games/MyGame", "C:/games/Other"].iter().map(PathBuf::from)),
            ..Default::default()
        };
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, None);
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        harness.get_by_label("File").hover();
        harness.run();
        harness.snapshot("chrome_file_menu");
    }

    /// The Open Recent submenu open, descended Menu -> File -> Open Recent (each level opens on hover -
    /// sharp-edges 3). Built over a model with two recents, so the submenu lists them and the new Clear
    /// Recently Opened item sits enabled at the foot below a separator. Guards the Clear affordance and
    /// the submenu contents.
    #[test]
    fn chrome_open_recent_menu_snapshot() {
        let _gpu = gpu_guard();
        let model = Model {
            recents: Recents::from_paths(["C:/games/MyGame", "C:/games/Other"].iter().map(PathBuf::from)),
            ..Default::default()
        };
        let mut harness = chrome_harness(egui::ThemePreference::Dark, egui::vec2(1100.0, 700.0), model, None);
        harness.run();
        harness.get_by_label("Menu").click();
        harness.run();
        harness.get_by_label("File").hover();
        harness.run();
        harness.get_by_label("Open Recent").hover();
        harness.run();
        harness.snapshot("chrome_open_recent_menu");
    }
}
