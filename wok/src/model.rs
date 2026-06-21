//! The editor model: the shell layout state - everything the action layer writes and the view reads.
//!
//! [`Model`] holds the [`Shell`] state (the active navigation view, plus the navigation panel's
//! visibility and dock side). It is the single value `action::handle` mutates and the view renders
//! from. Free of egui and the filesystem so the shell logic is unit testable without a window, and so
//! the chrome reads it the same way live and in the snapshot test.
//!
//! Single writer: `Shell`'s state is private and changes only through the `pub(crate)` mutators here,
//! which `crate::action::handle` calls. The view reads through the `pub` queries and never mutates -
//! it emits an [`Action`](crate::action::Action) instead. That seam is what later makes undo and redo
//! possible; today it carries the navigation actions - the active view, the panel's visibility, and
//! its dock side.

/// The navigation views, one per icon in the panel's bottom bar, split into the two scope groups the
/// divider separates: the project group (Scenes, Prefabs) is the same whichever scene is open; the
/// this-scene group (Instances, Lighting) is bound to the open scene. `title` is the view's canonical
/// name (the header label); the glyph mapping is a chrome concern and lives with the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavView {
    Scenes,
    Prefabs,
    /// The open scene's placements - the design's default landing view.
    #[default]
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

/// Which side of the editor the navigation panel docks to (the user's choice; a left-hand-keyboard,
/// right-hand-mouse setup is the reason it is configurable). Named `NavSide`, not `Side`, so it does
/// not clash with egui's `egui::panel::Side` where the view picks `SidePanel::left`/`::right`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavSide {
    #[default]
    Left,
    Right,
}

/// The editor's top-level state: how the shell around the content is arranged.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Model {
    /// The shell layout: the active navigation view, the panel's visibility and dock side. Tabs and
    /// the project lifecycle return in later slices.
    pub shell: Shell,
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
}

impl Default for Shell {
    fn default() -> Self {
        Shell { active_nav: NavView::default(), nav_visible: true, nav_side: NavSide::default() }
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_active_view_is_instances() {
        // The design's landing view: the open scene's placements.
        assert_eq!(Shell::default().active_nav(), NavView::Instances);
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
}
