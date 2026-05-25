use crate::authored::ChunkEagerness;
use crate::ids::{ChunkCoord, LightStateRef};

use super::region::RuntimeRegion;
use super::shape::{PhysicalHitbox, TriggerVolume, VisibleShape};

/// Runtime form of an authored `Chunk`. Produced by `slice_chunk` and consumed by downstream
/// systems (renderer, physics, trigger evaluator) per their per-system array.
///
/// `eagerness` is carried through from authored metadata so downstream crates can consult
/// the tag without round-tripping to authored data. The slicer treats all three eagerness
/// values identically (see plan section 5, "Eagerness-neutral"); runtime semantics for
/// Eager / Lazy / Vista live in `wok-content` and consumers (see plan section 8).
///
/// `surface_tag_table` is the interning side-table for `PhysicalHitbox::surface_tag`. An
/// empty `surface_tag` string in authored data interns to index 0.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkRuntime {
    pub coord: ChunkCoord,
    pub eagerness: ChunkEagerness,
    pub visible: Vec<VisibleShape>,
    pub hitboxes: Vec<PhysicalHitbox>,
    pub triggers: Vec<TriggerVolume>,
    pub regions: Vec<RuntimeRegion>,
    pub light_state: LightStateRef,
    pub surface_tag_table: Vec<String>,
}
