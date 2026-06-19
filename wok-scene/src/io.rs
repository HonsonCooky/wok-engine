//! File-level load and save for the three authored types (`Scene`, `Chunk`, `Prefab`).
//!
//! Each function operates on a single file. They do not resolve references, scan directories,
//! or compute paths from names; that is wok-content's and wok's job. Save uses pretty
//! printing so authored files diff cleanly in version control.
//!
//! Validation: the only in-file invariant the brief calls out is that a `Prefab`'s
//! `default_state` must name one of its states. Future invariants on `Scene` or `Chunk`
//! would slot into the same `validate_*` pattern next to the parser.

use std::path::Path;

use crate::chunk::Chunk;
use crate::error::{LoadError, SaveError};
use crate::prefab::Prefab;
use crate::scene::Scene;

// ---- Scene ----

pub fn load_scene(path: impl AsRef<Path>) -> Result<Scene, LoadError> {
    let path = path.as_ref();
    let bytes = read_file(path)?;
    parse_scene(&bytes, path)
}

pub fn save_scene(scene: &Scene, path: impl AsRef<Path>) -> Result<(), SaveError> {
    write_pretty_json(path.as_ref(), scene)
}

pub(crate) fn parse_scene(bytes: &[u8], path: &Path) -> Result<Scene, LoadError> {
    serde_json::from_slice::<Scene>(bytes).map_err(|source| LoadError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

// ---- Chunk ----

pub fn load_chunk(path: impl AsRef<Path>) -> Result<Chunk, LoadError> {
    let path = path.as_ref();
    let bytes = read_file(path)?;
    parse_chunk(&bytes, path)
}

pub fn save_chunk(chunk: &Chunk, path: impl AsRef<Path>) -> Result<(), SaveError> {
    write_pretty_json(path.as_ref(), chunk)
}

pub(crate) fn parse_chunk(bytes: &[u8], path: &Path) -> Result<Chunk, LoadError> {
    serde_json::from_slice::<Chunk>(bytes).map_err(|source| LoadError::Parse {
        path: path.to_path_buf(),
        source,
    })
}

// ---- Prefab ----

pub fn load_prefab(path: impl AsRef<Path>) -> Result<Prefab, LoadError> {
    let path = path.as_ref();
    let bytes = read_file(path)?;
    parse_prefab(&bytes, path)
}

pub fn save_prefab(prefab: &Prefab, path: impl AsRef<Path>) -> Result<(), SaveError> {
    write_pretty_json(path.as_ref(), prefab)
}

pub(crate) fn parse_prefab(bytes: &[u8], path: &Path) -> Result<Prefab, LoadError> {
    let prefab: Prefab = serde_json::from_slice(bytes).map_err(|source| LoadError::Parse {
        path: path.to_path_buf(),
        source,
    })?;
    validate_prefab(&prefab, path)?;
    Ok(prefab)
}

fn validate_prefab(prefab: &Prefab, path: &Path) -> Result<(), LoadError> {
    if !prefab.default_state_is_valid() {
        let known: Vec<&str> = prefab.states.iter().map(|s| s.name.as_str()).collect();
        return Err(LoadError::Validation {
            path: path.to_path_buf(),
            message: format!(
                "default_state {:?} does not match any of the prefab's states {:?}",
                prefab.default_state, known
            ),
        });
    }
    Ok(())
}

// ---- shared helpers ----

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
    use crate::chunk::{ChunkCoord, ChunkStreaming, Eagerness, Placement};
    use crate::math::{Aabb, Transform};
    use crate::prefab::{Prefab, PrefabState, Primitive, Shape};
    use crate::refs::{InstanceId, LightStateRef, MeshRef, PrefabRef, SurfaceTag};
    use crate::scene::{Region, Scene, StreamingDefaults};
    use glam::Vec3;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    // Unique temp paths per test invocation. tempfile would be nicer but is a new dep; this
    // single counter is sufficient for our local IO tests.
    fn unique_temp(prefix: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-scene-test-{}-{}-{}.json", prefix, pid, n))
    }

    fn sample_prefab() -> Prefab {
        Prefab {
            states: vec![
                PrefabState {
                    name: "default".into(),
                    shapes: vec![Shape {
                        primitive: Primitive::Cube,
                        transform: Transform::IDENTITY,
                        surface: Some(SurfaceTag::new("bark")),
                        is_hitbox: true,
                        is_visible: true,
                    }],
                    mesh: Some(MeshRef::new("oak_tree_lod0")),
                },
                PrefabState {
                    name: "destroyed".into(),
                    shapes: vec![],
                    mesh: None,
                },
            ],
            default_state: "default".into(),
        }
    }

    fn sample_chunk() -> Chunk {
        Chunk {
            coord: ChunkCoord::new(0, 0),
            placements: vec![Placement {
                prefab: PrefabRef::new("oak_tree"),
                instance_id: InstanceId(0),
                // Named on purpose: the file-level round trip covers the display name too.
                name: Some("the landmark oak".to_string()),
                transform: Transform::IDENTITY,
                state: None,
            }],
            streaming: ChunkStreaming {
                eagerness: Some(Eagerness::Eager),
                neighbors: vec![ChunkCoord::new(1, 0)],
                always_load_with: vec![],
            },
        }
    }

    fn sample_scene() -> Scene {
        Scene {
            name: "test_scene".into(),
            default_lighting: LightStateRef::new("noon"),
            regions: vec![Region {
                name: "courtyard".into(),
                bounds: Aabb::new(Vec3::ZERO, Vec3::splat(10.0)),
                lighting: LightStateRef::new("dawn"),
            }],
            default_streaming: StreamingDefaults {
                load_radius: 3,
                default_eagerness: Eagerness::Eager,
            },
            next_instance_id: InstanceId(42),
        }
    }

    // ---- Round-trip via real files ----

    #[test]
    fn save_and_load_prefab_round_trips() {
        let path = unique_temp("prefab");
        let p = sample_prefab();
        save_prefab(&p, &path).unwrap();
        let back = load_prefab(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, p);
    }

    #[test]
    fn save_and_load_chunk_round_trips() {
        let path = unique_temp("chunk");
        let c = sample_chunk();
        save_chunk(&c, &path).unwrap();
        let back = load_chunk(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, c);
    }

    #[test]
    fn save_and_load_scene_round_trips() {
        let path = unique_temp("scene");
        let s = sample_scene();
        save_scene(&s, &path).unwrap();
        let back = load_scene(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, s);
    }

    // ---- LoadError surfaces ----

    #[test]
    fn load_prefab_io_error_when_path_missing() {
        let path = unique_temp("missing");
        let err = load_prefab(&path).unwrap_err();
        match err {
            LoadError::Io { path: p, .. } => assert_eq!(p, path),
            other => panic!("expected Io error, got {:?}", other),
        }
    }

    #[test]
    fn load_prefab_parse_error_on_malformed_json() {
        let bytes = b"not json {";
        let err = parse_prefab(bytes, Path::new("synthetic.json")).unwrap_err();
        match err {
            LoadError::Parse { path, .. } => assert_eq!(path, Path::new("synthetic.json")),
            other => panic!("expected Parse error, got {:?}", other),
        }
    }

    #[test]
    fn load_prefab_validation_error_when_default_state_missing() {
        let p = Prefab {
            states: vec![PrefabState {
                name: "default".into(),
                shapes: vec![],
                mesh: None,
            }],
            default_state: "missing".into(),
        };
        let bytes = serde_json::to_vec(&p).unwrap();
        let err = parse_prefab(&bytes, Path::new("bad.json")).unwrap_err();
        match err {
            LoadError::Validation { path, message } => {
                assert_eq!(path, Path::new("bad.json"));
                assert!(
                    message.contains("missing"),
                    "expected message to mention the missing state name, got {:?}",
                    message
                );
            }
            other => panic!("expected Validation error, got {:?}", other),
        }
    }

    #[test]
    fn load_scene_parse_error_on_malformed_json() {
        let bytes = b"{ not valid";
        let err = parse_scene(bytes, Path::new("scene.json")).unwrap_err();
        assert!(matches!(err, LoadError::Parse { .. }));
    }

    #[test]
    fn load_chunk_parse_error_on_malformed_json() {
        let bytes = b"{";
        let err = parse_chunk(bytes, Path::new("0_0.json")).unwrap_err();
        assert!(matches!(err, LoadError::Parse { .. }));
    }
}
