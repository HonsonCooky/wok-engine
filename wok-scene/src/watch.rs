//! Filesystem watcher for hot reload of authored data.
//!
//! `Watcher` wraps the `notify` crate's recommended backend (ReadDirectoryChanges on Windows,
//! inotify on Linux, FSEvents on macOS) to watch a directory tree and report the raw paths that
//! changed. It is a thin drain over a channel: notify's background thread feeds events in, and
//! `poll` pulls whatever has arrived since the last call.
//!
//! The watcher reports paths only. It does not interpret which file changed, classify the change,
//! or decide what to reload - the game does that each frame after polling (see the HLD hot-reload
//! data flow). wok-scene neither maps a path back to a chunk nor distinguishes a created file from
//! a deleted one here; that belongs to wok-content and wok.
//!
//! Determinism: the watcher is a dev/authoring hook driven by real filesystem events and wall-
//! clock event delivery, so it is deliberately OUTSIDE the determinism contract the rest of
//! wok-scene honors. Hot reload feeds the authored -> runtime transform; it never participates in
//! simulation, replay, or save/load, so nondeterministic delivery here cannot affect deterministic
//! gameplay.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher as _};

/// Failure to start watching a path.
///
/// One variant by design: both creating the OS watcher and registering the path are in service of
/// "start watching this directory", and a consumer cannot act differently on the two sub-failures.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("failed to start watching {path:?}: {source}")]
    Start {
        path: PathBuf,
        #[source]
        source: notify::Error,
    },
}

/// A recursive directory watcher that reports raw changed paths.
///
/// Construct with `Watcher::new`; drain accumulated changes with `poll`. Dropping the `Watcher`
/// stops the underlying OS watch and ends its background thread.
pub struct Watcher {
    // The notify watcher owns the background thread that feeds `rx`. It is never read directly, but
    // must be kept alive: dropping it stops the watch and disconnects the channel.
    _inner: RecommendedWatcher,
    rx: Receiver<notify::Result<Event>>,
}

impl Watcher {
    /// Begin watching `path` and everything beneath it, recursively.
    ///
    /// Returns `WatchError::Start` if the OS watcher cannot be created or the path cannot be
    /// watched (for example, it does not exist or is not accessible).
    pub fn new(path: impl AsRef<Path>) -> Result<Self, WatchError> {
        let path = path.as_ref();
        let (tx, rx) = channel();
        // The handler runs on notify's background thread. `send` only fails once `rx` is dropped,
        // i.e. once this Watcher is gone, at which point dropping the event is the right thing.
        let mut inner = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })
        .map_err(|source| WatchError::Start {
            path: path.to_path_buf(),
            source,
        })?;
        inner
            .watch(path, RecursiveMode::Recursive)
            .map_err(|source| WatchError::Start {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Self { _inner: inner, rx })
    }

    /// Drain and return the paths that changed since the last call, deduplicated, without blocking.
    ///
    /// Order is first-seen across the drained events and carries no meaning beyond "these paths
    /// changed" (see the determinism note in the module docs). Backend errors are drained and
    /// discarded: a watcher that drops some events still reports the rest, the right failure mode
    /// for a best-effort authoring hook. An empty result means nothing changed (or the backend is
    /// briefly behind), never that watching has stopped.
    pub fn poll(&mut self) -> Vec<PathBuf> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        // `flatten` drops the `Err` events (backend hiccups), keeping only delivered events.
        for event in self.rx.try_iter().flatten() {
            for path in event.paths {
                if seen.insert(path.clone()) {
                    out.push(path);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("wok-scene-watch-{pid}-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn poll_reports_a_written_file() {
        let dir = unique_temp_dir();
        let mut watcher = Watcher::new(&dir).unwrap();

        // Give the OS watcher a beat to arm before the write; some backends need it.
        std::thread::sleep(Duration::from_millis(200));
        std::fs::write(dir.join("0_0.json"), b"{}").unwrap();

        // Filesystem events are asynchronous: poll until the path shows up or we time out. Match on
        // the file name only, since some backends canonicalize the watched root (e.g. macOS /var).
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut found = false;
        while Instant::now() < deadline {
            if watcher.poll().iter().any(|p| p.ends_with("0_0.json")) {
                found = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let _ = std::fs::remove_dir_all(&dir);
        assert!(found, "expected the written file to appear in a poll result");
    }

    #[test]
    fn poll_is_empty_with_no_changes() {
        let dir = unique_temp_dir();
        let mut watcher = Watcher::new(&dir).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        // Nothing was written, so there is nothing to drain.
        assert!(watcher.poll().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_on_missing_path_is_watch_error() {
        let missing = unique_temp_dir().join("does-not-exist");
        match Watcher::new(&missing) {
            Err(WatchError::Start { path, .. }) => assert_eq!(path, missing),
            Ok(_) => panic!("expected WatchError::Start watching a missing path"),
        }
        let _ = std::fs::remove_dir_all(missing.parent().unwrap());
    }
}
