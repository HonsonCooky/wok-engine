//! Scene manifest: defaults, regions, and the instance-id allocator.
//!
//! The manifest lives on disk as `scene.json` next to a flat set of `{x}_{z}.json` chunk
//! files. The chunk files are the source of truth for which chunks exist; the manifest does
//! not list them. This keeps add/remove of a chunk a single-file operation and avoids a
//! cross-file write whenever the editor stamps a new chunk.

use serde::{Deserialize, Serialize};

use crate::math::Aabb;
use crate::refs::{InstanceId, LightStateRef};

/// A volume within which a named lighting state applies (which carries fog and sky).
///
/// Regions are evaluated game-side; the engine just stores the volumes and lighting names.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub name: String,
    pub bounds: Aabb,
    pub lighting: LightStateRef,
}

/// Scene-wide streaming defaults.
///
/// `load_radius` is in chunks (not metres). `default_eagerness` is the fallback for chunks
/// whose own `ChunkStreaming::eagerness` is `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamingDefaults {
    pub load_radius: u32,
    pub default_eagerness: crate::chunk::Eagerness,
}

impl StreamingDefaults {
    /// The scene's render distance in metres: the streaming extent, `load_radius` chunks out at the
    /// engine's [`CHUNK_SIZE_M`](crate::CHUNK_SIZE_M) chunk size. This is the far bound of what the
    /// scene loads, so it is the natural far-plane distance - there is nothing to draw past it - and
    /// it is independent of fog: a scene may run with fog off and still bound its draw here.
    pub fn render_distance(&self) -> f32 {
        self.load_radius as f32 * crate::CHUNK_SIZE_M
    }
}

/// The scene manifest.
///
/// `next_instance_id` is the per-scene monotonic counter that `allocate_instance_id` advances.
/// Serializing the counter is what makes the monotonic-and-never-reused property survive a
/// save/load cycle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub name: String,
    pub default_lighting: LightStateRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<Region>,
    pub default_streaming: StreamingDefaults,
    pub next_instance_id: InstanceId,
}

impl Scene {
    /// Allocate the next per-scene instance id, advancing the counter by one.
    ///
    /// The counter is monotonic and never decrements; deleting a placement does not return
    /// its id. Panics if the u32 counter would overflow, which is not reachable in any
    /// realistic authoring session (4 billion placements per scene).
    pub fn allocate_instance_id(&mut self) -> InstanceId {
        let id = self.next_instance_id;
        let next = self
            .next_instance_id
            .0
            .checked_add(1)
            .expect("InstanceId counter overflowed u32");
        self.next_instance_id = InstanceId(next);
        id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::Eagerness;
    use glam::Vec3;

    fn sample_scene() -> Scene {
        Scene {
            name: "test_scene".into(),
            default_lighting: LightStateRef::new("noon"),
            regions: vec![Region {
                name: "courtyard".into(),
                bounds: Aabb::new(Vec3::new(-10.0, 0.0, -10.0), Vec3::new(10.0, 5.0, 10.0)),
                lighting: LightStateRef::new("dawn"),
            }],
            default_streaming: StreamingDefaults {
                load_radius: 3,
                default_eagerness: Eagerness::Eager,
            },
            next_instance_id: InstanceId(0),
        }
    }

    // ---- Region ----

    #[test]
    fn region_round_trips() {
        let r = Region {
            name: "atrium".into(),
            bounds: Aabb::new(Vec3::ZERO, Vec3::splat(8.0)),
            lighting: LightStateRef::new("dusk"),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: Region = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    // ---- StreamingDefaults ----

    #[test]
    fn streaming_defaults_round_trip() {
        let s = StreamingDefaults {
            load_radius: 4,
            default_eagerness: Eagerness::Lazy,
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: StreamingDefaults = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    #[allow(clippy::float_cmp)]
    fn render_distance_is_load_radius_times_chunk_size() {
        let s = StreamingDefaults { load_radius: 4, default_eagerness: Eagerness::Eager };
        assert_eq!(s.render_distance(), 4.0 * crate::CHUNK_SIZE_M);
        assert_eq!(s.render_distance(), 512.0); // 4 chunks x 128m
    }

    // ---- Scene ----

    #[test]
    fn scene_round_trips() {
        let s = sample_scene();
        let json = serde_json::to_string(&s).unwrap();
        let back: Scene = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn scene_round_trips_with_no_regions() {
        let s = Scene {
            regions: vec![],
            ..sample_scene()
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("regions"));
        let back: Scene = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    // ---- allocate_instance_id ----

    #[test]
    fn allocate_instance_id_is_monotonic() {
        let mut s = sample_scene();
        let a = s.allocate_instance_id();
        let b = s.allocate_instance_id();
        let c = s.allocate_instance_id();
        assert_eq!(a, InstanceId(0));
        assert_eq!(b, InstanceId(1));
        assert_eq!(c, InstanceId(2));
        assert_eq!(s.next_instance_id, InstanceId(3));
    }

    #[test]
    fn allocate_instance_id_never_reuses_after_many_allocations() {
        let mut s = sample_scene();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let id = s.allocate_instance_id();
            assert!(seen.insert(id), "InstanceId {:?} was reused", id);
        }
        // Strict-increase check too.
        let mut ids: Vec<InstanceId> = seen.into_iter().collect();
        ids.sort();
        for w in ids.windows(2) {
            assert!(w[1].0 > w[0].0);
        }
    }

    #[test]
    fn allocate_instance_id_resumes_from_serialized_counter() {
        // After a save/load cycle the counter should pick up exactly where it left off.
        let mut s = sample_scene();
        let _ = s.allocate_instance_id();
        let _ = s.allocate_instance_id();
        let json = serde_json::to_string(&s).unwrap();
        let mut back: Scene = serde_json::from_str(&json).unwrap();
        assert_eq!(back.next_instance_id, InstanceId(2));
        assert_eq!(back.allocate_instance_id(), InstanceId(2));
        assert_eq!(back.allocate_instance_id(), InstanceId(3));
    }
}
