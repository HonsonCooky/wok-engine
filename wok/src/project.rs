//! The current project: the editor's top-level "what am I authoring against" state, plus the open
//! semantics that decide whether a folder is a wok project.
//!
//! A project is a content-root folder (editor-design.md), the same model Zed uses for a source tree.
//! [`Project`] is the pure state - a root path, with the display name the title bar and status bar
//! show derived from it; the model holds it as `Option<Project>`, `None` being no project open. The
//! value itself is filesystem- and egui-free so the model stays unit-testable without a window.
//!
//! [`open`] is the validating constructor: a wok project is a folder holding a `scene.json`, so
//! opening one that has it succeeds and opening a folder without it - empty or not - is a
//! [`NotAProject`] error that writes nothing. Creating a starter project (generating content into an
//! empty folder) needs the engine content crates and the renderer, so it lands with the content bite;
//! this slice opens existing projects only. The filesystem touch is confined to [`open`] /
//! [`is_wok_project`], called from the frame loop, never from the pure `action::handle`.

use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::recent::Recents;

/// The scene manifest file whose presence marks a folder as a wok project. The v0 content layout
/// keeps it flat at the content root; the many-scenes folder rework (a later bite) revisits the
/// layout, but the manifest stays the open gate.
pub const SCENE_MANIFEST: &str = "scene.json";

/// An open project: a content-root folder. Construct it with [`open`], which validates the folder is
/// a wok project; the value then just carries the root, with [`name`](Project::name) deriving the
/// display name on demand. `new` skips the validation and is for the single writer to set the model
/// after the frame loop has already validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    root: PathBuf,
}

impl Project {
    /// Wrap an already-resolved root as an open project, without touching the filesystem. The single
    /// writer (`action::handle`) uses this to set the model after the frame loop has validated the
    /// folder with [`open`]; nothing else should, since it skips the wok-project check.
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

/// Open the project at `root`, validating it is a wok project. A folder holding a [`SCENE_MANIFEST`]
/// opens; a folder without one - empty or not - is a [`NotAProject`] error and writes nothing.
/// Creating a starter project into an empty folder needs the engine content crates and lands with the
/// content bite, so this slice never generates. The filesystem touch lives here, called from the
/// frame loop, never from the pure `action::handle`.
pub fn open(root: &Path) -> Result<Project, NotAProject> {
    if is_wok_project(root) {
        Ok(Project::new(root))
    } else {
        Err(NotAProject { root: root.to_path_buf() })
    }
}

/// Whether `root` is a loadable wok project: it holds a [`SCENE_MANIFEST`]. Reopen-last on launch
/// selects only these, so a recent whose folder was deleted or emptied is skipped rather than opened
/// into an error.
pub fn is_wok_project(root: &Path) -> bool {
    root.join(SCENE_MANIFEST).is_file()
}

/// The most-recent recent project satisfying `is_project`, most-recent first, or `None`. Pure over
/// the predicate so the most-recent-first selection is testable without a filesystem; the live caller
/// passes the on-disk [`is_wok_project`] check, so reopen-last skips a recent whose folder is gone and
/// never generates into one.
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

/// The picked folder is not a wok project: it holds no `scene.json`. Opening it would neither load
/// existing content nor (this slice) create new content, so the open is refused and nothing is
/// written. Carries the root for the message the status bar surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotAProject {
    root: PathBuf,
}

impl fmt::Display for NotAProject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "not a wok project: no {SCENE_MANIFEST} in {}", self.root.display())
    }
}

impl Error for NotAProject {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique temp directory path (not created), so the filesystem tests do not collide across
    /// parallel runs or repeated runs in the same process.
    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-project-test-{}-{}", std::process::id(), n))
    }

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

    // ---- open semantics (filesystem) ----

    #[test]
    fn open_an_existing_wok_project_succeeds() {
        // A folder holding a scene.json is a wok project: open succeeds and carries the root, and the
        // name is its leaf. The contents of scene.json are not read here - presence is the gate.
        let dir = unique_temp_dir();
        std::fs::create_dir_all(&dir).expect("create the project dir");
        std::fs::write(dir.join(SCENE_MANIFEST), b"{}").expect("seed a scene manifest");
        let project = open(&dir).expect("a folder with scene.json opens");
        assert_eq!(project.root(), dir);
        assert!(!project.name().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_an_empty_folder_errors_and_writes_nothing() {
        // An empty folder has no scene.json. This slice cannot generate a starter scene (that needs
        // the engine content crates), so it is NotAProject and nothing is written into the folder.
        let dir = unique_temp_dir();
        std::fs::create_dir_all(&dir).expect("create the empty dir");
        let message = match open(&dir) {
            Ok(_) => panic!("an empty folder is not a wok project this slice"),
            Err(err) => format!("{err}"),
        };
        assert!(message.contains("no scene.json"), "the error names the missing manifest: {message}");
        assert!(!dir.join(SCENE_MANIFEST).exists(), "open must not create a scene manifest");
        assert!(!dir.join("prefabs").exists(), "open must not scatter content into the folder");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_a_non_wok_folder_errors_and_writes_nothing() {
        // A non-empty folder without scene.json is some directory picked by mistake: open reports it
        // as not a wok project and writes nothing - it neither loads nor scatters content in.
        let dir = unique_temp_dir();
        std::fs::create_dir_all(&dir).expect("create the non-wok dir");
        std::fs::write(dir.join("README.md"), b"not a wok project").expect("seed a stray file");
        let message = match open(&dir) {
            Ok(_) => panic!("a non-wok folder must not open"),
            Err(err) => format!("{err}"),
        };
        assert!(message.contains("no scene.json"), "the error names the missing manifest: {message}");
        assert!(!dir.join(SCENE_MANIFEST).exists(), "open must not create a scene manifest");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn is_wok_project_checks_for_the_manifest() {
        let dir = unique_temp_dir();
        std::fs::create_dir_all(&dir).expect("create the dir");
        assert!(!is_wok_project(&dir), "a folder without scene.json is not a wok project");
        std::fs::write(dir.join(SCENE_MANIFEST), b"{}").expect("seed a scene manifest");
        assert!(is_wok_project(&dir), "a folder with scene.json is a wok project");
        let _ = std::fs::remove_dir_all(&dir);
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
        // A list of recents none of which is still a project (all deleted or emptied) starts empty.
        let recents = Recents::from_paths(["a", "b"].iter().map(PathBuf::from));
        assert_eq!(pick_startup(&recents, |_| false), None);
        // An empty recents list is also none, with no panic on the empty iterator.
        assert_eq!(pick_startup(&Recents::default(), |_| true), None);
    }
}
