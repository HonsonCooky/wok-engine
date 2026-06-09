//! File-level load and save for `LightState` and `LightCurve`.
//!
//! Each function operates on a single file, following wok-scene's `io` module: no directory
//! scanning, no name-to-path resolution (that is wok-content's job), pretty-printed JSON so
//! authored files diff cleanly. The crate takes no wok-scene dependency: it shares no type with
//! it. wok-scene's `LightStateRef` is a bare string name, and resolution from that name to a file
//! is by convention (`<name>.json`), not through an imported type.
//!
//! The name convention: a light state's name is its file stem, consistent with the name-based
//! references wok-scene uses. `load_light_state` returns `(name, state)` so a caller keying states
//! by name (the dynamic pool, later) gets the name without re-deriving it. `LightCurve` is not
//! referenced by name from wok-scene, so `load_light_curve` returns just the curve.
//!
//! Validation is intra-file only. For a curve: at least one keyframe, and strictly increasing
//! times so `sample` brackets unambiguously. A `LightState` has no intra-file invariant to check.

use std::path::Path;

use crate::curve::LightCurve;
use crate::error::{LoadError, SaveError};
use crate::state::LightState;

// ---- LightState ----

/// Load a light state, returning its file-stem name alongside the parsed state.
pub fn load_light_state(path: impl AsRef<Path>) -> Result<(String, LightState), LoadError> {
    let path = path.as_ref();
    let name = stem_name(path)?;
    let bytes = read_file(path)?;
    let state = parse_light_state(&bytes, path)?;
    Ok((name, state))
}

pub fn save_light_state(state: &LightState, path: impl AsRef<Path>) -> Result<(), SaveError> {
    write_pretty_json(path.as_ref(), state)
}

pub(crate) fn parse_light_state(bytes: &[u8], path: &Path) -> Result<LightState, LoadError> {
    serde_json::from_slice::<LightState>(bytes).map_err(|source| LoadError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

// ---- LightCurve ----

pub fn load_light_curve(path: impl AsRef<Path>) -> Result<LightCurve, LoadError> {
    let path = path.as_ref();
    let bytes = read_file(path)?;
    parse_light_curve(&bytes, path)
}

pub fn save_light_curve(curve: &LightCurve, path: impl AsRef<Path>) -> Result<(), SaveError> {
    write_pretty_json(path.as_ref(), curve)
}

pub(crate) fn parse_light_curve(bytes: &[u8], path: &Path) -> Result<LightCurve, LoadError> {
    let curve: LightCurve = serde_json::from_slice(bytes).map_err(|source| LoadError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    validate_curve(&curve, path)?;
    Ok(curve)
}

fn validate_curve(curve: &LightCurve, path: &Path) -> Result<(), LoadError> {
    if curve.keyframes.is_empty() {
        return Err(LoadError::Validation {
            path: path.to_path_buf(),
            message: "a light curve must have at least one keyframe".into(),
        });
    }
    for pair in curve.keyframes.windows(2) {
        if pair[1].time <= pair[0].time {
            return Err(LoadError::Validation {
                path: path.to_path_buf(),
                message: format!(
                    "keyframe times must be strictly increasing, but {} follows {}",
                    pair[1].time, pair[0].time
                ),
            });
        }
    }
    Ok(())
}

// ---- shared helpers ----

/// Derive a light state's name from its file path: the file stem as UTF-8. A path with no stem
/// (e.g. `..`) or a non-UTF-8 stem is a `BadName` error rather than a silent empty name.
fn stem_name(path: &Path) -> Result<String, LoadError> {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(ToString::to_string)
        .ok_or_else(|| LoadError::BadName { path: path.to_path_buf() })
}

fn read_file(path: &Path) -> Result<Vec<u8>, LoadError> {
    std::fs::read(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn write_pretty_json<T: serde::Serialize>(path: &Path, value: &T) -> Result<(), SaveError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|source| SaveError::Serialize {
        path: path.to_path_buf(),
        source,
    })?;
    std::fs::write(path, bytes).map_err(|source| SaveError::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::Keyframe;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    // Unique temp paths per test invocation, matching wok-scene's io tests: a single counter is
    // enough for these local IO tests without pulling in a tempfile dependency.
    fn unique_temp(stem: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-light-test-{}-{}-{}.json", stem, pid, n))
    }

    fn sample_curve() -> LightCurve {
        LightCurve {
            keyframes: vec![
                Keyframe { time: 0.0, state: LightState::default() },
                Keyframe { time: 12.0, state: LightState::default() },
            ],
            looping: true,
        }
    }

    // ---- LightState round-trip and name derivation ----

    #[test]
    fn save_and_load_light_state_round_trips() {
        let path = unique_temp("noon");
        let s = LightState::default();
        save_light_state(&s, &path).unwrap();
        let (name, back) = load_light_state(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, s);
        // The derived name is the file stem, not a field of the file.
        assert!(name.starts_with("wok-light-test-noon-"));
    }

    #[test]
    fn light_state_name_is_the_file_stem() {
        // A hand-built path so the stem is exactly known.
        let dir = std::env::temp_dir();
        let path = dir.join("dawn.json");
        save_light_state(&LightState::default(), &path).unwrap();
        let (name, _) = load_light_state(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(name, "dawn");
    }

    #[test]
    fn load_light_state_io_error_when_missing() {
        let path = unique_temp("missing");
        match load_light_state(&path).unwrap_err() {
            LoadError::Io { path: p, .. } => assert_eq!(p, path),
            other => panic!("expected Io error, got {:?}", other),
        }
    }

    #[test]
    fn load_light_state_parse_error_on_malformed_json() {
        let err = parse_light_state(b"not json {", Path::new("noon.json")).unwrap_err();
        match err {
            LoadError::Parse { path, .. } => assert_eq!(path, Path::new("noon.json")),
            other => panic!("expected Parse error, got {:?}", other),
        }
    }

    // ---- LightCurve round-trip and validation ----

    #[test]
    fn save_and_load_light_curve_round_trips() {
        let path = unique_temp("daynight");
        let c = sample_curve();
        save_light_curve(&c, &path).unwrap();
        let back = load_light_curve(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, c);
    }

    #[test]
    fn load_light_curve_rejects_empty_keyframes() {
        let c = LightCurve { keyframes: vec![], looping: false };
        let bytes = serde_json::to_vec(&c).unwrap();
        match parse_light_curve(&bytes, Path::new("empty.json")).unwrap_err() {
            LoadError::Validation { message, .. } => assert!(message.contains("at least one")),
            other => panic!("expected Validation error, got {:?}", other),
        }
    }

    #[test]
    fn load_light_curve_rejects_non_increasing_times() {
        let c = LightCurve {
            keyframes: vec![
                Keyframe { time: 5.0, state: LightState::default() },
                Keyframe { time: 5.0, state: LightState::default() },
            ],
            looping: false,
        };
        let bytes = serde_json::to_vec(&c).unwrap();
        match parse_light_curve(&bytes, Path::new("bad.json")).unwrap_err() {
            LoadError::Validation { message, .. } => assert!(message.contains("strictly increasing")),
            other => panic!("expected Validation error, got {:?}", other),
        }
    }
}
