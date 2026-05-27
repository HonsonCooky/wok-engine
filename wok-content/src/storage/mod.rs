//! Storage layer over loaded mesh data. CPU representation (step 3) gives primitives a
//! producer-side type; GPU buffers and upload (step 4) give the worker pipeline a target.

pub mod mesh;

pub use mesh::{MeshCpu, MeshGpu, MeshVertex, upload};
