//! Pure transformation from authored chunks to runtime arrays. See plan section 5 for the
//! algorithm and the six properties this implementation guarantees.

use std::collections::HashMap;

use crate::authored::{Chunk, Prefab, PrefabState, Shape, TerrainData};
use crate::error::SliceError;
use crate::ids::{ChunkCoord, PrefabId};
use crate::runtime::{
    ChunkRuntime, PhysicalHitbox, RuntimeRegion, RuntimeTerrain, TriggerVolume, VisibleShape,
};

/// Resolves prefab identifiers to authored prefab data. Implementations must be deterministic
/// for the lifetime of a slice call: the same id passed twice must produce the same result.
pub trait PrefabLookup {
    fn get(&self, id: &PrefabId) -> Option<&Prefab>;
}

impl<S: std::hash::BuildHasher> PrefabLookup for HashMap<PrefabId, Prefab, S> {
    fn get(&self, id: &PrefabId) -> Option<&Prefab> {
        HashMap::get(self, id)
    }
}

/// Color applied to a `VisibleShape` when the authored `Shape.visual_color` is `None`. White
/// means "no tint" for any renderer that multiplies through a material color.
const DEFAULT_VISUAL_COLOR: [f32; 3] = [1.0, 1.0, 1.0];

/// Slice authored chunk data into per-system runtime arrays.
///
/// All transforms in the output are chunk-local. World coordinates are obtained at consumer
/// sites by composing with `chunk.coord.to_world_offset()`. This is the property the
/// parallel-worlds multiplayer model depends on: the same authored chunk file produces
/// bit-identical runtime arrays on every client, regardless of where the chunk sits in the
/// world. The slicer never touches `chunk.coord` for transform composition.
pub fn slice_chunk(
    chunk: &Chunk,
    prefabs: &dyn PrefabLookup,
) -> Result<ChunkRuntime, SliceError> {
    // Pass 1: resolve prefab+state for each placement, validate shape flags, count by
    // classification. Any error aborts here; pass 2 sees only valid data.
    let mut resolved: Vec<&PrefabState> = Vec::with_capacity(chunk.placements.len());
    let mut n_visible = 0usize;
    let mut n_hitboxes = 0usize;
    let mut n_triggers = 0usize;
    for (placement_idx, placement) in chunk.placements.iter().enumerate() {
        let prefab = prefabs
            .get(&placement.prefab)
            .ok_or_else(|| SliceError::UnknownPrefab(placement.prefab.clone()))?;
        let state = prefab
            .states
            .iter()
            .find(|s| s.name == placement.state)
            .ok_or_else(|| SliceError::UnknownState {
                prefab: placement.prefab.clone(),
                state: placement.state.clone(),
            })?;
        for (shape_idx, shape) in state.shapes.iter().enumerate() {
            validate_shape_flags(shape, placement_idx, shape_idx)?;
            match (shape.is_hitbox, shape.is_visible) {
                (true, true) => {
                    n_visible += 1;
                    n_hitboxes += 1;
                }
                (true, false) => n_triggers += 1,
                (false, true) => n_visible += 1,
                (false, false) => unreachable!("validate_shape_flags rejected this case"),
            }
        }
        resolved.push(state);
    }

    // Pass 2: slice. Output vectors are pre-sized to their exact final length so no
    // growth-doubling occurs.
    let mut visible = Vec::with_capacity(n_visible);
    let mut hitboxes = Vec::with_capacity(n_hitboxes);
    let mut triggers = Vec::with_capacity(n_triggers);
    // 16 covers every realistic chunk by a wide margin (typical < 10 distinct surface
    // tags); pre-sizing here closes the "single allocation per output vector" property
    // without needing a separate count-distinct sub-pass.
    let mut surface_intern = StringInterner::with_capacity(16);

    for (placement_idx, placement) in chunk.placements.iter().enumerate() {
        let state = resolved[placement_idx];
        // placement.transform is chunk-local. We compose it with each shape's
        // prefab-local transform to get a chunk-local matrix. We deliberately do NOT
        // compose chunk.coord.to_world_offset() here; consumers do that at draw or query
        // time. This is the position-independence property (plan section 5 property 5).
        let placement_m = placement.transform.to_mat4();
        let source_placement = placement_idx as u32;
        for shape in &state.shapes {
            let local_transform = placement_m * shape.transform.to_mat4();
            match (shape.is_hitbox, shape.is_visible) {
                (true, true) => {
                    visible.push(VisibleShape {
                        primitive: shape.primitive,
                        local_transform,
                        color: shape.visual_color.unwrap_or(DEFAULT_VISUAL_COLOR),
                        source_placement,
                    });
                    hitboxes.push(PhysicalHitbox {
                        primitive: shape.primitive,
                        local_transform,
                        surface_tag: surface_intern
                            .intern(shape.surface_tag.as_deref().unwrap_or("")),
                        source_placement,
                    });
                }
                (true, false) => {
                    triggers.push(TriggerVolume {
                        primitive: shape.primitive,
                        local_transform,
                        trigger_id: shape
                            .trigger_id
                            .as_ref()
                            .expect("validate_shape_flags ensured trigger_id is set")
                            .clone(),
                        source_placement,
                    });
                }
                (false, true) => {
                    visible.push(VisibleShape {
                        primitive: shape.primitive,
                        local_transform,
                        color: shape.visual_color.unwrap_or(DEFAULT_VISUAL_COLOR),
                        source_placement,
                    });
                }
                (false, false) => unreachable!("validate_shape_flags rejected this case"),
            }
        }
    }

    // Count and emission must agree; a refactor that broke this invariant would silently
    // trigger a Vec reallocation without test failure. Cheap defense.
    debug_assert_eq!(visible.len(), n_visible);
    debug_assert_eq!(hitboxes.len(), n_hitboxes);
    debug_assert_eq!(triggers.len(), n_triggers);

    let mut regions = Vec::with_capacity(chunk.regions.len());
    for r in &chunk.regions {
        regions.push(RuntimeRegion {
            name: r.name.clone(),
            local_bounds: r.bounds,
            purpose: r.purpose.clone(),
        });
    }

    // v0.2.0: terrain pass. Merge authored terrain surface tags into the same intern table
    // (preserving prefab-first ordering), then rewrite per-cell surface indices through the
    // resulting remap. Position-independent: nothing here reads chunk.coord except the
    // overflow-error context. See plan section 5 "Slicing terrain" and the determinism
    // properties (#1, #4, #5).
    let terrain = match &chunk.terrain {
        None => None,
        Some(authored) => Some(slice_terrain(authored, &mut surface_intern, chunk.coord)?),
    };

    Ok(ChunkRuntime {
        coord: chunk.coord,
        eagerness: chunk.metadata.eagerness,
        visible,
        hitboxes,
        triggers,
        regions,
        light_state: chunk.light_state.clone(),
        surface_tag_table: surface_intern.into_vec(),
        terrain,
    })
}

/// Merge authored terrain surface tags into the runtime intern table and produce a
/// `RuntimeTerrain` whose surface indices reference the merged table. Authored tags are
/// appended to the intern table in the order they appear in `authored.surface_tags` (which
/// the save path keeps alphabetically sorted, see `authored::terrain::write_sibling`).
/// Returns `SliceError::TerrainSurfaceTableOverflow` if the merge would push the runtime
/// table past `u16::MAX` entries.
fn slice_terrain(
    authored: &TerrainData,
    intern: &mut StringInterner,
    coord: ChunkCoord,
) -> Result<RuntimeTerrain, SliceError> {
    let prefab_tag_count = intern.len();
    let mut remap: Vec<u16> = Vec::with_capacity(authored.surface_tags.len());
    for tag in &authored.surface_tags {
        let runtime_idx = intern.intern(tag);
        let runtime_idx_u16 =
            u16::try_from(runtime_idx).map_err(|_| SliceError::TerrainSurfaceTableOverflow {
                coord,
                prefab_tag_count,
                terrain_tag_count: authored.surface_tags.len(),
            })?;
        remap.push(runtime_idx_u16);
    }

    // Rewrite per-cell surface indices through the remap. An out-of-range authored index
    // (which would indicate corrupt in-memory TerrainData; loader-validated data cannot
    // produce this) passes through unchanged: the cell then references whatever happens to
    // be at that runtime-table position. Cheaper than threading another error variant for a
    // case that the loader already rejects.
    let surface_indices: Box<[u16]> = authored
        .surface_indices
        .iter()
        .map(|&authored_idx| {
            remap
                .get(usize::from(authored_idx))
                .copied()
                .unwrap_or(authored_idx)
        })
        .collect();

    Ok(RuntimeTerrain {
        heights: authored.heights.clone(),
        surface_indices,
        width: TerrainData::CELLS_PER_AXIS,
        vertical_range_meters: authored.vertical_range_meters,
    })
}

fn validate_shape_flags(
    shape: &Shape,
    placement_index: usize,
    shape_index: usize,
) -> Result<(), SliceError> {
    match (shape.is_hitbox, shape.is_visible) {
        (true, false) if shape.trigger_id.is_none() => Err(SliceError::InvalidShape {
            placement_index,
            shape_index,
            reason: "hitbox-only shape (trigger volume) requires a trigger_id".to_string(),
        }),
        (false, false) => Err(SliceError::InvalidShape {
            placement_index,
            shape_index,
            reason: "shape has neither is_hitbox nor is_visible set".to_string(),
        }),
        _ => Ok(()),
    }
}

/// First-appearance string interner backed by a `Vec<String>`. Determinism property (plan
/// section 5 property 1) requires this be FIFO: the i-th unique string presented to `intern`
/// returns index `i`. Linear scan via `Vec::iter().position`. Surface tag counts are small
/// (handful per chunk), so O(n) per intern is acceptable.
struct StringInterner {
    table: Vec<String>,
}

impl StringInterner {
    fn with_capacity(cap: usize) -> Self {
        Self {
            table: Vec::with_capacity(cap),
        }
    }

    fn intern(&mut self, s: &str) -> u32 {
        if let Some(idx) = self.table.iter().position(|x| x == s) {
            idx as u32
        } else {
            let idx = self.table.len() as u32;
            self.table.push(s.to_string());
            idx
        }
    }

    fn len(&self) -> usize {
        self.table.len()
    }

    fn into_vec(self) -> Vec<String> {
        self.table
    }
}
