//! wok-mesh: mesh data, generation, and GPU upload.
//!
//! The geometry layer the renderer draws, in two halves. The pure CPU half: a mesh data type
//! ([`MeshCpu`]) plus generators that produce one from the engine's placeholder primitives and
//! from a chunk's terrain heightmap; deterministic, fully testable without a GPU, and dependent
//! only on wok-scene (plus glam). The GPU half: [`MeshGpu`] and the upload path (`crate::gpu`),
//! which is where the crate takes its wok-platform dependency for the pinned wgpu re-export.
//! Mesh-name resolution and a GLTF loader arrive with real assets, later.
//!
//! ## Two producers, one data type
//!
//! - [`primitive_mesh`] (and the per-shape [`cube`], [`plane`], [`ellipsoid`], [`cylinder`],
//!   [`capsule`]) turn a [`Primitive`] into a unit-sized mesh.
//! - [`terrain_mesh`] triangulates a chunk [`Heightmap`] into a mesh in the heightmap's frame.
//!
//! Both yield a [`MeshCpu`]: a vertex list (position + outward normal, the minimum cel-shaded
//! rendering needs) and a triangle index list, single consistent winding (counter-clockwise front
//! faces). UVs, vertex colour and per-vertex surface tags are deliberately absent: the renderer's
//! vertex format is not pinned yet, and terrain surface colouring is the renderer's concern (it
//! samples the heightmap's surface grid), so baking either in now would be guessing.
//!
//! ## The unit-primitive convention (cross-crate contract)
//!
//! This is the load-bearing reason the crate exists in the layer it does. A primitive mesh is a
//! *unit* shape; its size and placement come from the placement transform at draw time, exactly as
//! collision bounds do. The convention is owned by wok-scene - the unit half-extent
//! (`wok_scene::UNIT_HALF_EXTENT`, 0.5) and each primitive's unit bounds (`Primitive::unit_aabb`); see
//! `designs/high-level-design.md`, wok-scene section. Every volumetric primitive is inscribed in the
//! cube spanning `+/-0.5` on each axis, and `Plane` is the flat square of that half-extent at
//! `y = 0`. The per-shape generators read that one constant for vertex placement, and the
//! cross-convention test (see `primitive` tests) asserts a generated mesh's bounds equal
//! `Primitive::unit_aabb` for the same shape - so drawn meshes and colliders agree against a single
//! source, not by parallel restatement.
//!
//! A consequence worth stating: a radius-0.5 capsule inscribed in the unit cube has zero
//! cylinder-body height, so the unit `Capsule` mesh *is* the unit sphere. See [`capsule`]. The one
//! generator deliberately outside the convention is [`capsule_mesh`], a true capsule parameterized
//! in metres that pairs with wok-physics's parameterized `Capsule`; its docs carry the distinction.
//!
//! ## Determinism (canon contract)
//!
//! Generation is deterministic: identical inputs produce a bitwise-identical [`MeshCpu`]. Every
//! generator is a fixed sequence of arithmetic with no RNG, no wall-clock and no parallelism. Mesh
//! data feeds rendering, not simulation, so this is not strictly required by the replay harness, but
//! it is cheap and keeps the tests exact. Upload (`crate::gpu`) is rendering-side residency and not
//! part of simulation state.
//!
//! ## Errors
//!
//! There are none: generation is total over valid inputs, and upload has no reportable failure mode
//! (see `crate::gpu`). A degenerate tessellation parameter (zero segments or rings) is clamped to
//! the coarsest valid mesh rather than reported, and a [`Heightmap`] is already validated by its own
//! constructor. Per `designs/project-canon.md` a `thiserror` enum is added only when a genuine
//! failure mode exists; none does here, so the crate exposes no `Error`.
//!
//! [`Primitive`]: wok_scene::Primitive
//! [`Heightmap`]: wok_scene::Heightmap

pub mod capsule;
pub mod cube;
pub mod cylinder;
pub mod ellipsoid;
pub mod gpu;
pub mod mesh;
pub mod primitive;
mod surface;
pub mod terrain;

pub use capsule::{capsule, capsule_mesh};
pub use cube::{cube, plane};
pub use cylinder::cylinder;
pub use ellipsoid::ellipsoid;
pub use gpu::{MeshGpu, VERTEX_LAYOUT, VERTEX_STRIDE};
pub use mesh::{MeshCpu, Vertex};
pub use primitive::{DEFAULT_RINGS, DEFAULT_SEGMENTS, primitive_mesh};
pub use terrain::terrain_mesh;
