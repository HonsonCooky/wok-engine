//! Procedural terrain mesh generation. Consumes a `ChunkRuntime` whose `terrain` field is
//! `Some(_)` and produces a `MeshCpu` ready for GPU upload. Plan section 5.1 worker pipeline,
//! sections 9.17 and 9.18 for the pinned constraints.
//!
//! - **9.17 - slot-owned**: terrain meshes are not registered, not deduplicated, and not
//!   stored in `ContentSystem.meshes`. They live on `ResidentChunk.gpu.terrain` and drop
//!   with the slot.
//! - **9.18 - NW-SE triangulation locked**: each 1m x 1m quad splits along the diagonal
//!   from `(i, j)` to `(i+1, j+1)` (low-x low-z corner to high-x high-z corner) so the
//!   same `TerrainData` produces byte-identical `MeshCpu` across two slices, runs, and
//!   clients. Runtime "pick the shorter diagonal" heuristics are forbidden; if such a
//!   feature ever lands it ships as an authored property on `TerrainData`, not as a
//!   runtime decision.

pub mod mesh;
pub mod palette;

pub use mesh::generate_mesh;
