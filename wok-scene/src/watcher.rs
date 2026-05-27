//! File-system watcher for the content directory. Wraps `notify-debouncer-full` with a
//! ~100ms debounce, classifies raw filesystem events into typed `FileEvent`s based on the
//! content-directory layout from plan section 4, and exposes a drain-style `poll` API.
//!
//! The watcher does not re-parse files, does not dedupe against actual content changes, and
//! does not emit anything on startup. Those are the consumer's concerns.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use notify::{EventKind, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{DebounceEventResult, Debouncer, RecommendedCache, new_debouncer};

use crate::ids::ChunkCoord;

/// Classified file-system event. The watcher only emits these for paths under the watched
/// content root; files outside the documented prefixes (prefabs/, scenes/, lights/) are
/// silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileEvent {
    PrefabChanged(PathBuf),
    PrefabRemoved(PathBuf),
    SceneManifestChanged(PathBuf),
    ChunkChanged {
        scene_dir: PathBuf,
        coord: ChunkCoord,
    },
    ChunkRemoved {
        scene_dir: PathBuf,
        coord: ChunkCoord,
    },
    LightStateChanged(PathBuf),
    Error(String),
}

/// Debounced filesystem watcher rooted at a content directory. Dropping the watcher
/// terminates the background notify thread.
pub struct FileWatcher {
    // Holds the debouncer alive so its background thread keeps running. Field-prefixed with
    // `_` because we never read it directly - all access is through the events channel.
    _debouncer: Debouncer<RecommendedWatcher, RecommendedCache>,
    events: Receiver<FileEvent>,
}

impl FileWatcher {
    /// Spawn a watcher rooted at `content_root`. The directory must exist and be readable.
    /// `content_root` is canonicalized so the watcher and classifier agree on path shape
    /// (notify reports canonicalized paths on most platforms).
    pub fn new(content_root: &Path) -> Result<Self, std::io::Error> {
        let canonical = content_root.canonicalize()?;
        let (tx, rx) = channel::<FileEvent>();
        let classify_root = canonical.clone();
        let mut debouncer = new_debouncer(
            Duration::from_millis(100),
            None,
            move |result: DebounceEventResult| {
                handle_result(&classify_root, result, &tx);
            },
        )
        .map_err(io_err_from_notify)?;
        debouncer
            .watch(&canonical, RecursiveMode::Recursive)
            .map_err(io_err_from_notify)?;
        Ok(FileWatcher {
            _debouncer: debouncer,
            events: rx,
        })
    }

    /// Drain accumulated events. Returns whatever has been classified since the last call;
    /// returns an empty `Vec` if nothing has happened.
    pub fn poll(&mut self) -> Vec<FileEvent> {
        let mut out = Vec::new();
        while let Ok(event) = self.events.try_recv() {
            out.push(event);
        }
        out
    }
}

fn io_err_from_notify(e: notify::Error) -> std::io::Error {
    std::io::Error::other(e)
}

fn handle_result(content_root: &Path, result: DebounceEventResult, tx: &Sender<FileEvent>) {
    match result {
        Ok(events) => {
            for event in events {
                let removed = matches!(event.event.kind, EventKind::Remove(_));
                for path in &event.paths {
                    if let Some(classified) = classify(content_root, path, removed) {
                        let _ = tx.send(classified);
                    }
                }
            }
        }
        Err(errors) => {
            for e in errors {
                let _ = tx.send(FileEvent::Error(format!("watcher error: {e}")));
            }
        }
    }
}

/// Map a raw filesystem path under `content_root` to a typed `FileEvent`, or `None` if the
/// path does not match any documented prefix.
///
/// Slicing the relative path into string components (skipping anything that is not a normal
/// component) keeps the matcher OS-independent: `\` vs `/` separator differences disappear
/// before we compare against the prefix names.
fn classify(content_root: &Path, path: &Path, removed: bool) -> Option<FileEvent> {
    let rel = path.strip_prefix(content_root).ok()?;
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();

    match parts.as_slice() {
        ["prefabs", filename] if json_stem(filename).is_some() => Some(if removed {
            FileEvent::PrefabRemoved(path.to_owned())
        } else {
            FileEvent::PrefabChanged(path.to_owned())
        }),
        ["lights", filename] if json_stem(filename).is_some() => {
            Some(FileEvent::LightStateChanged(path.to_owned()))
        }
        ["scenes", scene_name, filename] => {
            let scene_dir = content_root.join("scenes").join(scene_name);
            // v0.2.0: heightmap sibling binaries coalesce into ChunkChanged with the parsed
            // coord. No separate variant - the consumer treats a heightmap touch the same
            // way it treats a chunk JSON touch (re-load, re-slice). A removed binary still
            // emits ChunkChanged (the JSON may have been updated to drop the reference and
            // the consumer's re-load is what reconciles).
            if let Some(stem) = heightmap_stem(filename) {
                return Some(match parse_chunk_stem(stem) {
                    Some(coord) => FileEvent::ChunkChanged { scene_dir, coord },
                    None => FileEvent::Error(format!(
                        "heightmap file {} has unparseable name (expected `{{i}}_{{j}}.heightmap.bin`)",
                        path.display()
                    )),
                });
            }
            let stem = json_stem(filename)?;
            if filename.eq_ignore_ascii_case("scene.json") {
                Some(FileEvent::SceneManifestChanged(path.to_owned()))
            } else {
                match parse_chunk_stem(stem) {
                    Some(coord) => Some(if removed {
                        FileEvent::ChunkRemoved { scene_dir, coord }
                    } else {
                        FileEvent::ChunkChanged { scene_dir, coord }
                    }),
                    None => Some(FileEvent::Error(format!(
                        "chunk file {} has unparseable name (expected `{{i}}_{{j}}.json`)",
                        path.display()
                    ))),
                }
            }
        }
        _ => None,
    }
}

/// Return the chunk-stem (filename minus the `.heightmap.bin` suffix) if the file matches
/// the heightmap pattern under ASCII case-insensitive matching, otherwise `None`. Filenames
/// like `foo.HEIGHTMAP.BIN` are recognized identically to `foo.heightmap.bin` so the watcher
/// behaves consistently on case-insensitive filesystems.
fn heightmap_stem(filename: &str) -> Option<&str> {
    const SUFFIX_LEN: usize = ".heightmap.bin".len();
    if filename.len() <= SUFFIX_LEN {
        return None;
    }
    let split_at = filename.len() - SUFFIX_LEN;
    let (stem, ext) = filename.split_at(split_at);
    if ext.eq_ignore_ascii_case(".heightmap.bin") {
        Some(stem)
    } else {
        None
    }
}

/// Return the stem (filename minus extension) if the file has a `.json` extension under
/// ASCII case-insensitive matching, otherwise `None`. Case-insensitive so the watcher
/// behaves consistently on case-insensitive filesystems (NTFS, default APFS).
fn json_stem(filename: &str) -> Option<&str> {
    let path = std::path::Path::new(filename);
    let ext = path.extension()?;
    if !ext.eq_ignore_ascii_case("json") {
        return None;
    }
    path.file_stem()?.to_str()
}

/// Parse the stem (filename minus `.json`) of a chunk file as `{i}_{j}`. Splits on the first
/// `_`; both halves must parse as `i32`. Negative coordinates work because each half is
/// parsed independently (e.g., `-1_-2` -> ("-1", "-2") -> ChunkCoord::new(-1, -2)).
fn parse_chunk_stem(stem: &str) -> Option<ChunkCoord> {
    let (i_str, j_str) = stem.split_once('_')?;
    let x: i32 = i_str.parse().ok()?;
    let z: i32 = j_str.parse().ok()?;
    Some(ChunkCoord::new(x, z))
}
