//! The editor model: the open project plus the shell layout state - everything the action layer
//! writes and the view reads.
//!
//! [`Model`] bundles the open [`Project`](crate::project::Project) with the [`Shell`] state (the
//! navigation panel's dock and visibility, the open tabs, and which is active). It is the single
//! value `action::handle` mutates and the view renders from. Free of egui and the filesystem so the
//! shell logic is unit testable without a window.

use crate::project::Project;
use crate::recent::Recents;

/// The editor's top-level state: what is open, and how the shell around it is arranged.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Model {
    /// The open project (a content-root folder), or none.
    pub project: Project,
    /// The shell layout: the navigation panel and the open tabs.
    pub shell: Shell,
    /// The recently opened projects (most-recent first), persisted across runs. Seeded from disk at
    /// startup and written back by the action layer when it changes (`crate::recent`).
    pub recents: Recents,
}

impl Model {
    /// Build a model around an initial project, with a default shell (panel shown on the left, no
    /// tabs open) and an empty recent-projects list. The startup edge seeds recents from disk after
    /// construction (`crate::app`).
    pub fn new(project: Project) -> Model {
        Model { project, shell: Shell::default(), recents: Recents::default() }
    }
}

/// Which side the navigation panel docks to. Left suits a left-hand-keyboard, right-hand-mouse
/// setup, but the choice is the user's (editor-design.md), flipped through the View menu.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    #[default]
    Left,
    Right,
}

/// An opaque, stable handle to an open tab. Stable across other tabs opening and closing, so the
/// view can name a tab to select or close without tracking shifting positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(u64);

/// Which kind of edit context a tab hosts. Scene is the spatial hub - one per project, bound to the
/// project's scene; prefab and lighting kinds join as those views are built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    /// The scene viewport: the god-cam over the project's chunk content.
    Scene,
}

/// One open edit context: a stable id, a title, and which kind of surface it hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: TabId,
    pub title: String,
    pub kind: TabKind,
}

/// The shell layout state: the navigation panel (docked side and visibility) and the open tabs
/// (which exist and which is active).
///
/// Every field is mutated only through the methods below, which `action::handle` calls - the single
/// writer. The methods hold one invariant the view relies on: `active` always names a tab that
/// exists, or is `None` when no tabs are open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    tabs: Vec<Tab>,
    active: Option<TabId>,
    nav_visible: bool,
    nav_side: Side,
    /// Monotonic source of tab ids and untitled-tab numbers; only ever increases, so a closed tab's
    /// id is never reused by a later one.
    next_id: u64,
}

impl Default for Shell {
    fn default() -> Shell {
        Shell { tabs: Vec::new(), active: None, nav_visible: true, nav_side: Side::Left, next_id: 0 }
    }
}

impl Shell {
    // ---- queries (the view reads these) ----

    /// The open tabs, left to right.
    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    /// The active tab's id, or `None` when no tabs are open.
    pub fn active(&self) -> Option<TabId> {
        self.active
    }

    /// The active tab, or `None` when no tabs are open.
    pub fn active_tab(&self) -> Option<&Tab> {
        let id = self.active?;
        self.tabs.iter().find(|t| t.id == id)
    }

    /// Whether the navigation panel is shown.
    pub fn nav_visible(&self) -> bool {
        self.nav_visible
    }

    /// Which side the navigation panel docks to.
    pub fn nav_side(&self) -> Side {
        self.nav_side
    }

    // ---- mutations (only action::handle calls these) ----

    /// Open a new tab with this title and kind, and make it active. Returns its id. The id source
    /// only ever climbs, so a closed tab's id is never reused by a later one.
    pub(crate) fn open_tab(&mut self, title: String, kind: TabKind) -> TabId {
        let id = TabId(self.next_id);
        self.next_id += 1;
        self.tabs.push(Tab { id, title, kind });
        self.active = Some(id);
        id
    }

    /// Open the Scene tab, or focus it when one is already open. There is one scene per project, so
    /// opening it again - from the content browser, or the auto-open at project load - focuses the
    /// existing tab rather than duplicating it. Returns its id.
    pub(crate) fn open_or_focus_scene(&mut self) -> TabId {
        match self.tabs.iter().find(|t| t.kind == TabKind::Scene).map(|t| t.id) {
            Some(id) => {
                self.active = Some(id);
                id
            }
            None => self.open_tab("Scene".to_string(), TabKind::Scene),
        }
    }

    /// Close the tab with this id. If it was active, activate a sensible neighbour: the tab that
    /// slides into its slot (the one to its right), or the new last tab when it was rightmost, or
    /// none when it was the only tab. A no-op when no tab has this id.
    pub(crate) fn close_tab(&mut self, id: TabId) {
        let Some(idx) = self.tabs.iter().position(|t| t.id == id) else { return };
        self.tabs.remove(idx);
        if self.active == Some(id) {
            self.active = self.tabs.get(idx).or_else(|| self.tabs.last()).map(|t| t.id);
        }
    }

    /// Close every open tab and clear the active tab. The tabs are project-scoped, so closing the
    /// project clears them; the nav panel's dock side and visibility are workspace preferences, not
    /// project content, and are left alone. `next_id` keeps climbing, so a later tab never reuses a
    /// closed tab's id.
    pub(crate) fn close_all_tabs(&mut self) {
        self.tabs.clear();
        self.active = None;
    }

    /// Make the tab with this id active. A no-op when no tab has this id, so the active tab always
    /// stays valid.
    pub(crate) fn select_tab(&mut self, id: TabId) {
        if self.tabs.iter().any(|t| t.id == id) {
            self.active = Some(id);
        }
    }

    /// Flip the navigation panel between shown and hidden.
    pub(crate) fn toggle_nav(&mut self) {
        self.nav_visible = !self.nav_visible;
    }

    /// Dock the navigation panel to `side`.
    pub(crate) fn set_nav_side(&mut self, side: Side) {
        self.nav_side = side;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The id of the tab at `index`. Panics if absent - a test-only assumption that the tab exists.
    fn tab_id(shell: &Shell, index: usize) -> TabId {
        shell.tabs()[index].id
    }

    #[test]
    fn open_tab_adds_and_activates_it() {
        let mut shell = Shell::default();
        shell.open_tab("Scene".to_string(), TabKind::Scene);
        assert_eq!(shell.tabs().len(), 1);
        assert_eq!(shell.active(), Some(tab_id(&shell, 0)));
    }

    #[test]
    fn opening_a_second_tab_activates_the_new_one() {
        let mut shell = Shell::default();
        shell.open_tab("a".to_string(), TabKind::Scene);
        shell.open_tab("b".to_string(), TabKind::Scene);
        assert_eq!(shell.tabs().len(), 2);
        assert_eq!(shell.active(), Some(tab_id(&shell, 1)));
    }

    #[test]
    fn closing_the_active_tab_activates_its_right_neighbour() {
        let mut shell = Shell::default();
        shell.open_tab("a".to_string(), TabKind::Scene); // index 0
        shell.open_tab("b".to_string(), TabKind::Scene); // index 1
        shell.open_tab("c".to_string(), TabKind::Scene); // index 2, active
        let middle = tab_id(&shell, 1);
        shell.select_tab(middle);
        shell.close_tab(middle);
        assert_eq!(shell.tabs().len(), 2);
        // The tab that was to the right (originally index 2) slid into index 1 and is now active.
        assert_eq!(shell.active(), Some(tab_id(&shell, 1)));
    }

    #[test]
    fn closing_the_active_rightmost_tab_activates_the_new_last() {
        let mut shell = Shell::default();
        shell.open_tab("a".to_string(), TabKind::Scene);
        shell.open_tab("b".to_string(), TabKind::Scene); // index 1, active (rightmost)
        let right = tab_id(&shell, 1);
        shell.close_tab(right);
        assert_eq!(shell.tabs().len(), 1);
        assert_eq!(shell.active(), Some(tab_id(&shell, 0)));
    }

    #[test]
    fn closing_the_last_remaining_tab_clears_the_active_tab() {
        let mut shell = Shell::default();
        shell.open_tab("only".to_string(), TabKind::Scene);
        let only = tab_id(&shell, 0);
        shell.close_tab(only);
        assert!(shell.tabs().is_empty());
        assert_eq!(shell.active(), None);
    }

    #[test]
    fn closing_an_inactive_tab_leaves_the_active_tab_alone() {
        let mut shell = Shell::default();
        shell.open_tab("a".to_string(), TabKind::Scene); // index 0
        shell.open_tab("b".to_string(), TabKind::Scene); // index 1, active
        let active = tab_id(&shell, 1);
        let inactive = tab_id(&shell, 0);
        shell.close_tab(inactive);
        assert_eq!(shell.tabs().len(), 1);
        assert_eq!(shell.active(), Some(active));
    }

    #[test]
    fn select_tab_switches_the_active_tab() {
        let mut shell = Shell::default();
        shell.open_tab("a".to_string(), TabKind::Scene); // index 0
        shell.open_tab("b".to_string(), TabKind::Scene); // index 1, active
        let first = tab_id(&shell, 0);
        shell.select_tab(first);
        assert_eq!(shell.active(), Some(first));
    }

    #[test]
    fn close_all_tabs_clears_the_strip_and_active() {
        let mut shell = Shell::default();
        shell.open_tab("a".to_string(), TabKind::Scene);
        shell.open_tab("b".to_string(), TabKind::Scene);
        shell.close_all_tabs();
        assert!(shell.tabs().is_empty());
        assert_eq!(shell.active(), None);
    }

    #[test]
    fn open_or_focus_scene_opens_once_then_focuses_the_same_tab() {
        let mut shell = Shell::default();
        let first = shell.open_or_focus_scene();
        assert_eq!(shell.tabs().len(), 1);
        // A second open-scene focuses the existing Scene tab rather than duplicating it.
        let again = shell.open_or_focus_scene();
        assert_eq!(again, first, "the same Scene tab is focused, not a new one");
        assert_eq!(shell.tabs().len(), 1, "there is one scene per project");
        assert_eq!(shell.active(), Some(first));
    }

    #[test]
    fn open_or_focus_scene_refocuses_an_unfocused_scene_tab() {
        let mut shell = Shell::default();
        let scene = shell.open_or_focus_scene();
        let other = shell.open_tab("Other".to_string(), TabKind::Scene); // a second tab steals focus
        assert_eq!(shell.active(), Some(other));
        // open-scene focuses the FIRST Scene tab (the one bound to the project's scene).
        assert_eq!(shell.open_or_focus_scene(), scene);
        assert_eq!(shell.active(), Some(scene));
    }
}
