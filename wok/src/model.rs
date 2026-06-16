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

/// One open edit context. Minimal for now - a stable id and a title, with placeholder content; real
/// context kinds (scene, prefab, lighting) and the navigation binding arrive with those surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub id: TabId,
    pub title: String,
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

    /// Open a new untitled placeholder tab and make it active. Returns its id.
    pub(crate) fn open_tab(&mut self) -> TabId {
        let id = TabId(self.next_id);
        let title = format!("Untitled {}", self.next_id + 1);
        self.next_id += 1;
        self.tabs.push(Tab { id, title });
        self.active = Some(id);
        id
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
