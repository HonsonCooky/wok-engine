//! The current project: the editor's top-level "what am I authoring against" state.
//!
//! A project is a content-root folder (editor-design.md), the same model Zed uses for a source
//! tree. This module is the pure state - none open, or one open at a root path - plus the display
//! name the title bar and status bar show. Opening a project only establishes its root here;
//! loading and rendering its scene content return with the rendering piece. Free of egui and the
//! filesystem so the state is unit testable on its own.

use std::path::{Path, PathBuf};

/// The current project: either none is open, or one is open at a content-root folder.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum Project {
    /// No project open; the editor shows an empty viewport and the File menu is the way in.
    #[default]
    None,
    /// A project open at the given content root.
    Open { root: PathBuf },
}

impl Project {
    /// Open a project at `root`. The path is stored as written; existence and content loading are a
    /// later piece's concern, so this never touches the filesystem.
    pub fn open(root: impl Into<PathBuf>) -> Project {
        Project::Open { root: root.into() }
    }

    /// The open project's root folder, or `None` when no project is open.
    pub fn root(&self) -> Option<&Path> {
        match self {
            Project::None => None,
            Project::Open { root } => Some(root),
        }
    }

    /// The folder name shown in the window title and status bar, or `None` when no project is open.
    /// It is the root's final component (the folder's own name); a path with no final component (a
    /// drive or filesystem root) falls back to the whole path as written, so an open project always
    /// has a non-empty name to show.
    pub fn display_name(&self) -> Option<String> {
        self.root().map(display_name_of)
    }
}

/// The display name for a content root: its final component, or the whole path when there is none.
fn display_name_of(root: &Path) -> String {
    root.file_name().map_or_else(
        || root.to_string_lossy().into_owned(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_no_project() {
        let project = Project::default();
        assert_eq!(project.root(), None);
        assert_eq!(project.display_name(), None);
    }

    #[test]
    fn open_records_the_root() {
        let project = Project::open("games/unstitched");
        assert_eq!(project.root(), Some(Path::new("games/unstitched")));
    }

    #[test]
    fn display_name_is_the_final_folder_component() {
        // Forward slashes are path separators on every platform Rust targets, so a nested path
        // yields its leaf, and a bare folder yields itself.
        assert_eq!(Project::open("games/unstitched").display_name().as_deref(), Some("unstitched"));
        assert_eq!(Project::open("unstitched").display_name().as_deref(), Some("unstitched"));
    }

    #[test]
    fn display_name_falls_back_to_the_whole_path_for_a_root() {
        // A filesystem/drive root has no final component; the name is then the path itself, never
        // empty.
        let name = Project::open("/").display_name();
        assert!(name.is_some_and(|name| !name.is_empty()), "a root path still shows something");
    }
}
