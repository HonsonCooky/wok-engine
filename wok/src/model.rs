//! The editor model: the open project, the recent-projects list, and the shell layout state -
//! everything the action layer writes and the view reads.
//!
//! [`Model`] holds the open [`Project`](crate::project::Project) (or `None`), the persisted
//! [`Recents`](crate::recent::Recents), and the [`Shell`] state (the active navigation view, the open
//! tabs, and the navigation panel's visibility and dock side). It is the single value `action::handle`
//! mutates and the view renders from. Free of egui and the filesystem so the shell logic is unit
//! testable without a window, and so the chrome reads it the same way live and in the snapshot test -
//! the disk I/O for opening a project and persisting recents lives in `crate::project` /
//! `crate::recent`, called from the frame loop, never here.
//!
//! Single writer: `Shell`'s state is private and changes only through the `pub(crate)` mutators here,
//! which `crate::action::handle` calls. The view reads through the `pub` queries and never mutates -
//! it emits an [`Action`](crate::action::Action) instead. That seam is what later makes undo and redo
//! possible; today it carries the navigation actions - the active view, the panel's visibility, and
//! its dock side - the open tabs (open or focus, switch, close), plus the project lifecycle (open,
//! open recent, close).

use crate::project::Project;
use crate::recent::Recents;

/// The navigation views, one per icon in the panel's bottom bar, split into the two scope groups the
/// divider separates: the project group (Scenes, Prefabs) is the same whichever scene is open; the
/// this-scene group (Instances, Lighting) is bound to the open scene. `title` is the view's canonical
/// name (the header label); the glyph mapping is a chrome concern and lives with the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavView {
    /// The project's scenes (levels). The default landing view: with no scene open yet, this is where
    /// you pick one to open as a tab.
    #[default]
    Scenes,
    Prefabs,
    /// The open scene's placements.
    Instances,
    Lighting,
}

impl NavView {
    /// The view's canonical name, shown as the panel header label.
    pub fn title(self) -> &'static str {
        match self {
            NavView::Scenes => "Scenes",
            NavView::Prefabs => "Prefabs",
            NavView::Instances => "Instances",
            NavView::Lighting => "Lighting",
        }
    }
}

/// How the Instances view orders the open scene's placements - the panel header's one contextual
/// control for that view. `Group` buckets placements under their prefab as a collapsible tree (the
/// default, the way you read a scene: "how many of each thing"); `Flat` is a single flat list sorted
/// A-Z by row label. View state, not project content: it changes how the same placements are shown,
/// never what is authored, so it lives on the [`Shell`] beside the active view, not on disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstanceSort {
    /// Group placements by prefab, one collapsible group row per prefab with its instance count. The
    /// default.
    #[default]
    Group,
    /// A single flat list, every placement as one row, sorted A-Z by its display label.
    Flat,
}

/// Which side of the editor the navigation panel docks to (the user's choice; a left-hand-keyboard,
/// right-hand-mouse setup is the reason it is configurable). Named `NavSide`, not `Side`, so it does
/// not clash with egui's `egui::panel::Side` where the view picks `SidePanel::left`/`::right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavSide {
    #[default]
    Left,
    Right,
}

/// One open edit context in the tab bar, over the editor area. A tab is opened from content (a
/// navigation row), never blank (editor-design.md: no blank-tab affordance). This bite opens only
/// scenes; the enum is left open so the prefab and lighting edit contexts a later bite adds are new
/// variants here, not a reshape of the tab model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tab {
    /// An open scene, keyed by its name (the `assets/scenes/<name>` folder). The name is the tab's
    /// identity: opening the same scene again focuses the open tab rather than duplicating it
    /// ([`Shell::open_tab`]).
    Scene(String),
}

impl Tab {
    /// The tab's display title - what the tab cell shows and the editor well names. For a scene, its
    /// name.
    pub fn title(&self) -> &str {
        match self {
            Tab::Scene(name) => name,
        }
    }
}

/// The editor's top-level state: what project is open, the recent-projects list, and how the shell
/// around the content is arranged.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Model {
    /// The open project (a content-root folder), or `None` when none is open. Set by the single
    /// writer after the frame loop validates the folder (`crate::project`, `crate::action`); the
    /// chrome reads it for the window title and the status bar.
    pub project: Option<Project>,
    /// The shell layout: the active navigation view, the open tabs and which is active, and the
    /// panel's visibility and dock side.
    pub shell: Shell,
    /// The recently opened projects (most-recent first), persisted across runs. Seeded from disk at
    /// startup and written back by the action layer when it changes (`crate::recent`).
    pub recents: Recents,
}

/// The shell layout state: the active navigation view, whether the navigation panel is visible, and
/// which side it docks to. Every field is private and mutated only through the `pub(crate)` methods
/// below, which `action::handle` calls - the single writer. The view reads through the `pub` queries
/// and never assigns. `Default` is hand-written because `nav_visible` starts `true`, which `derive`
/// cannot express.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    active_nav: NavView,
    nav_visible: bool,
    nav_side: NavSide,
    /// The open tabs, in bar order (left to right). Empty when nothing is open.
    tabs: Vec<Tab>,
    /// The active tab, as an index into `tabs`, or `None` when no tab is open. The mutators keep it in
    /// step with `tabs`, holding the invariant "`Some` exactly when `tabs` is non-empty".
    active_tab: Option<usize>,
    /// How the Instances view orders the active scene's placements (group-by-prefab or flat A-Z). The
    /// panel header's contextual control for that view sets it; the body reads it to pick the layout.
    instance_sort: InstanceSort,
}

impl Default for Shell {
    fn default() -> Self {
        Shell {
            active_nav: NavView::default(),
            nav_visible: true,
            nav_side: NavSide::default(),
            tabs: Vec::new(),
            active_tab: None,
            instance_sort: InstanceSort::default(),
        }
    }
}

impl Shell {
    // ---- queries (the view reads these) ----

    /// The navigation view shown in the panel.
    pub fn active_nav(&self) -> NavView {
        self.active_nav
    }

    /// Whether the navigation panel is shown. Hidden, the view column spans the full editor width.
    pub fn nav_visible(&self) -> bool {
        self.nav_visible
    }

    /// Which side the navigation panel docks to.
    pub fn nav_side(&self) -> NavSide {
        self.nav_side
    }

    /// The open tabs, in bar order. The tab bar renders these; the active one is [`active_tab`].
    ///
    /// [`active_tab`]: Self::active_tab
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// The active tab as an index into [`tabs`](Self::tabs), or `None` when no tab is open.
    pub fn active_tab(&self) -> Option<usize> {
        self.active_tab
    }

    /// How the Instances view orders its placements (group-by-prefab or flat A-Z). The Instances body
    /// reads this to pick its layout, and the header control marks the active mode from it.
    pub fn instance_sort(&self) -> InstanceSort {
        self.instance_sort
    }

    // ---- mutations (only action::handle calls these) ----

    /// Switch the panel to show `view`.
    pub(crate) fn select_nav(&mut self, view: NavView) {
        self.active_nav = view;
    }

    /// Show the navigation panel if hidden, hide it if shown.
    pub(crate) fn toggle_nav(&mut self) {
        self.nav_visible = !self.nav_visible;
    }

    /// Dock the navigation panel to `side`.
    pub(crate) fn set_nav_side(&mut self, side: NavSide) {
        self.nav_side = side;
    }

    /// Set how the Instances view orders its placements.
    pub(crate) fn set_instance_sort(&mut self, sort: InstanceSort) {
        self.instance_sort = sort;
    }

    /// Open `tab`, or focus it if an equal tab is already open - the no-duplicate rule (one obvious
    /// way to a given edit context). Either way the tab ends up active.
    pub(crate) fn open_tab(&mut self, tab: Tab) {
        match self.tabs.iter().position(|open| *open == tab) {
            Some(existing) => self.active_tab = Some(existing),
            None => {
                self.tabs.push(tab);
                self.active_tab = Some(self.tabs.len() - 1);
            }
        }
    }

    /// Make the tab at `index` active (out of range is a no-op).
    pub(crate) fn select_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active_tab = Some(index);
        }
    }

    /// Close the tab at `index` (out of range is a no-op). Closing the active tab moves focus to the
    /// tab that slides into its slot, or the new last tab when the active one was at the end; closing
    /// the final tab leaves none active. A tab closed before the active one shifts the active index
    /// down by one, so focus stays on the same tab.
    pub(crate) fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.active_tab = match self.active_tab {
            // The last tab is gone: nothing is active.
            _ if self.tabs.is_empty() => None,
            // Closed the active tab: focus the tab that slid into its slot, or the new last tab if the
            // active one was at the end.
            Some(active) if active == index => Some(index.min(self.tabs.len() - 1)),
            // Closed a tab before the active one: its index shifts down by one to track the same tab.
            Some(active) if active > index => Some(active - 1),
            // Closed a tab after the active one (or nothing was active): the active index is unchanged.
            other => other,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_active_view_is_scenes() {
        // The landing view: with no scene open yet, Scenes is where you pick one to open.
        assert_eq!(Shell::default().active_nav(), NavView::Scenes);
    }

    #[test]
    fn select_nav_switches_the_active_view() {
        let mut shell = Shell::default();
        shell.select_nav(NavView::Scenes);
        assert_eq!(shell.active_nav(), NavView::Scenes);
        shell.select_nav(NavView::Lighting);
        assert_eq!(shell.active_nav(), NavView::Lighting);
    }

    #[test]
    fn the_nav_panel_starts_visible_and_docked_left() {
        let shell = Shell::default();
        assert!(shell.nav_visible());
        assert_eq!(shell.nav_side(), NavSide::Left);
    }

    #[test]
    fn toggle_nav_flips_visibility() {
        let mut shell = Shell::default();
        shell.toggle_nav();
        assert!(!shell.nav_visible());
        shell.toggle_nav();
        assert!(shell.nav_visible());
    }

    #[test]
    fn set_nav_side_docks_to_either_side() {
        let mut shell = Shell::default();
        shell.set_nav_side(NavSide::Right);
        assert_eq!(shell.nav_side(), NavSide::Right);
        shell.set_nav_side(NavSide::Left);
        assert_eq!(shell.nav_side(), NavSide::Left);
    }

    #[test]
    fn the_default_instance_sort_is_group_by_prefab() {
        // The way you read a scene first: how many of each thing. The flat A-Z list is the alternative,
        // not the landing.
        assert_eq!(Shell::default().instance_sort(), InstanceSort::Group);
    }

    #[test]
    fn set_instance_sort_switches_the_instances_ordering() {
        let mut shell = Shell::default();
        shell.set_instance_sort(InstanceSort::Flat);
        assert_eq!(shell.instance_sort(), InstanceSort::Flat);
        shell.set_instance_sort(InstanceSort::Group);
        assert_eq!(shell.instance_sort(), InstanceSort::Group);
    }

    // ---- tabs ----

    #[test]
    fn a_default_shell_has_no_tabs() {
        let shell = Shell::default();
        assert!(shell.tabs().is_empty());
        assert_eq!(shell.active_tab(), None);
    }

    #[test]
    fn open_tab_opens_then_focuses_without_duplicating() {
        let mut shell = Shell::default();
        shell.open_tab(Tab::Scene("village".into()));
        shell.open_tab(Tab::Scene("dungeon".into()));
        assert_eq!(shell.tabs(), &[Tab::Scene("village".into()), Tab::Scene("dungeon".into())]);
        assert_eq!(shell.active_tab(), Some(1), "the most-recently opened tab is active");

        // Re-opening an already-open scene focuses its tab rather than adding a second.
        shell.open_tab(Tab::Scene("village".into()));
        assert_eq!(shell.tabs().len(), 2, "no duplicate tab");
        assert_eq!(shell.active_tab(), Some(0), "the re-opened tab is focused");
    }

    #[test]
    fn select_tab_focuses_an_open_tab_and_ignores_out_of_range() {
        let mut shell = Shell::default();
        shell.open_tab(Tab::Scene("a".into()));
        shell.open_tab(Tab::Scene("b".into()));
        shell.select_tab(0);
        assert_eq!(shell.active_tab(), Some(0));
        shell.select_tab(9); // out of range: leaves the active tab unchanged
        assert_eq!(shell.active_tab(), Some(0));
    }

    #[test]
    fn closing_the_active_tab_focuses_the_neighbour_that_slides_in() {
        let mut shell = Shell::default();
        for name in ["a", "b", "c"] {
            shell.open_tab(Tab::Scene(name.into()));
        }
        shell.select_tab(1); // b active
        shell.close_tab(1);
        // c slid into slot 1 and takes focus.
        assert_eq!(shell.tabs(), &[Tab::Scene("a".into()), Tab::Scene("c".into())]);
        assert_eq!(shell.active_tab(), Some(1));
    }

    #[test]
    fn closing_the_active_last_tab_focuses_the_new_last() {
        let mut shell = Shell::default();
        for name in ["a", "b", "c"] {
            shell.open_tab(Tab::Scene(name.into()));
        }
        // c (index 2, the last) is active after the opens; closing it falls back to the new last.
        shell.close_tab(2);
        assert_eq!(shell.tabs(), &[Tab::Scene("a".into()), Tab::Scene("b".into())]);
        assert_eq!(shell.active_tab(), Some(1), "b, the new last tab");
    }

    #[test]
    fn closing_a_tab_before_the_active_one_keeps_focus_on_the_same_tab() {
        let mut shell = Shell::default();
        for name in ["a", "b", "c"] {
            shell.open_tab(Tab::Scene(name.into()));
        }
        shell.select_tab(2); // c active
        shell.close_tab(0); // close a, before the active one
        // The active index shifts down so it still points at c.
        assert_eq!(shell.tabs(), &[Tab::Scene("b".into()), Tab::Scene("c".into())]);
        assert_eq!(shell.active_tab(), Some(1));
    }

    #[test]
    fn closing_the_last_remaining_tab_leaves_none_active() {
        let mut shell = Shell::default();
        shell.open_tab(Tab::Scene("only".into()));
        shell.close_tab(0);
        assert!(shell.tabs().is_empty());
        assert_eq!(shell.active_tab(), None);
    }

    #[test]
    fn close_tab_out_of_range_is_a_no_op() {
        let mut shell = Shell::default();
        shell.open_tab(Tab::Scene("a".into()));
        shell.close_tab(5);
        assert_eq!(shell.tabs().len(), 1);
        assert_eq!(shell.active_tab(), Some(0));
    }

    #[test]
    fn a_default_model_has_no_project_and_no_recents() {
        // The editor starts with nothing open and an empty MRU list; the startup edge then seeds
        // recents from disk and may reopen the last project (`crate::main`).
        let model = Model::default();
        assert!(model.project.is_none());
        assert!(model.recents.is_empty());
    }
}
