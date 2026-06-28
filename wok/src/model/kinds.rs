//! The shell's enumerated vocabulary: the navigation views, how the Instances view orders its
//! placements, which side the panel docks to, and the kind of an open tab. Small value types the
//! [`Shell`](super::Shell) is built from; split out from the state itself so the state file holds the
//! single-writer machinery and these hold the choices it ranges over. Re-exported from `super` (the
//! model module), so the rest of the crate names them `crate::model::NavView` and so on.

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
