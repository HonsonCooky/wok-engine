//! The current project: the editor's top-level "what am I authoring against" state.
//!
//! A project is just a content-root folder (editor-design.md), the same model Zed uses for a source
//! tree. Opening one has no gate and no error: any folder the user picks becomes the project, and a
//! folder with no `assets/` is simply an empty project (HLD "content conventions and integrity"). wok
//! writes only inside `assets/`, and only on save (a later bite), so opening touches nothing and there
//! is nothing to validate - the open is a pure state change, applied through `action::handle` like any
//! other action, with no filesystem step in the frame loop.
//!
//! [`Project`] is the pure state - a root path, with the display name the title bar and status bar
//! show derived from it; the model holds it as `Option<Project>`, `None` being no project open. The
//! value itself is filesystem- and egui-free so the model stays unit-testable without a window.

use std::path::{Path, PathBuf};

use crate::recent::Recents;

/// An open project: a content-root folder. [`new`](Project::new) wraps a root; the value then just
/// carries it, with [`name`](Project::name) deriving the display name on demand. There is no
/// validating constructor - opening a folder has no gate, so whatever the picker returned is wrapped
/// as-is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    root: PathBuf,
}

impl Project {
    /// Wrap a content-root folder as the open project, without touching the filesystem. Opening has no
    /// gate, so any picked folder is wrapped as-is; the single writer (`action::handle`) calls this.
    pub fn new(root: impl Into<PathBuf>) -> Project {
        Project { root: root.into() }
    }

    /// The open project's content root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder name shown in the window title and status bar: the root's final component, or the
    /// whole path when it has none (a drive or filesystem root), so an open project always shows a
    /// non-empty name.
    pub fn name(&self) -> String {
        display_name_of(self.root())
    }
}

/// The most-recent recent project satisfying `is_project`, most-recent first, or `None`. Pure over the
/// predicate so the most-recent-first selection is testable without a filesystem; the live caller
/// passes a "the folder still exists" check (`Path::is_dir`), so reopen-last skips a recent whose
/// folder was deleted or moved while opening any folder that is still present.
pub fn pick_startup(recents: &Recents, is_project: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    recents.paths().iter().find(|root| is_project(root)).cloned()
}

/// The display name for a content root: its final component, or the whole path when there is none.
/// Shared by [`Project::name`] and the Open Recent menu, so a project reads the same in the title
/// bar, the status bar, and the recents list.
pub fn display_name_of(root: &Path) -> String {
    root.file_name().map_or_else(
        || root.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- display name (pure) ----

    #[test]
    fn name_is_the_final_folder_component() {
        // Forward slashes are path separators on every platform Rust targets, so a nested path yields
        // its leaf and a bare folder yields itself.
        assert_eq!(Project::new("games/unstitched").name(), "unstitched");
        assert_eq!(Project::new("unstitched").name(), "unstitched");
    }

    #[test]
    fn name_falls_back_to_the_whole_path_for_a_root() {
        // A filesystem/drive root has no final component; the name is then the path itself, never
        // empty, so an open project always shows something.
        assert!(!Project::new("/").name().is_empty(), "a root path still shows something");
    }

    #[test]
    fn new_records_the_root() {
        assert_eq!(Project::new("games/unstitched").root(), Path::new("games/unstitched"));
    }

    // ---- reopen-last (pure over the predicate) ----

    #[test]
    fn pick_startup_takes_the_most_recent_matching_project() {
        // Most-recent first: c, b, a. Only a and b qualify, so reopen-last picks the more recent of
        // those (b), skipping the non-project c at the front.
        let recents = Recents::from_paths(["c", "b", "a"].iter().map(PathBuf::from));
        let picked = pick_startup(&recents, |p| p == Path::new("a") || p == Path::new("b"));
        assert_eq!(picked, Some(PathBuf::from("b")));
    }

    #[test]
    fn pick_startup_is_none_when_nothing_qualifies() {
        // A list of recents none of which is still present (all deleted or moved) starts empty.
        let recents = Recents::from_paths(["a", "b"].iter().map(PathBuf::from));
        assert_eq!(pick_startup(&recents, |_| false), None);
        // An empty recents list is also none, with no panic on the empty iterator.
        assert_eq!(pick_startup(&Recents::default(), |_| true), None);
    }
}
