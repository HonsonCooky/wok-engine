//! Storage layer over loaded mesh data. CPU representation lands in step 3 alongside
//! primitives (primitives produce `MeshCpu`); GPU buffers and upload helpers land in step 4.

pub mod mesh;

pub use mesh::{MeshCpu, MeshVertex};
