//! The editor model: the shell layout state - everything the action layer writes and the view reads.
//!
//! [`Model`] holds the [`Shell`] state (for this slice, just which navigation view is active). It is
//! the single value `action::handle` mutates and the view renders from. Free of egui and the
//! filesystem so the shell logic is unit testable without a window, and so the chrome reads it the
//! same way live and in the snapshot test.
//!
//! Single writer: `Shell`'s state is private and changes only through the `pub(crate)` mutators here,
//! which `crate::action::handle` calls. The view reads through the `pub` queries and never mutates -
//! it emits an [`Action`](crate::action::Action) instead. That seam is what later makes undo and redo
//! possible; today it carries one action, switching the active navigation view.

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

/// The editor's top-level state: how the shell around the content is arranged.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Model {
    /// The shell layout. For this slice, the active navigation view; tabs, dock side, and visibility
    /// return in later slices.
    pub shell: Shell,
}

/// The shell layout state. For this slice it is the active navigation view; the field is private and
/// mutated only through the `pub(crate)` method below, which `action::handle` calls - the single
/// writer. The view reads it through `active_nav` and never assigns it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Shell {
    active_nav: NavView,
}

impl Shell {
    // ---- queries (the view reads these) ----

    /// The navigation view shown in the panel.
    pub fn active_nav(&self) -> NavView {
        self.active_nav
    }

    // ---- mutations (only action::handle calls these) ----

    /// Switch the panel to show `view`.
    pub(crate) fn select_nav(&mut self, view: NavView) {
        self.active_nav = view;
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
}
