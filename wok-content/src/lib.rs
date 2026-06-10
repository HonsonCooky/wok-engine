//! wok-content: chunk lifecycle and the authored-to-runtime transform.
//!
//! This crate owns HLD data-flow state 3, the runtime-arrays form of a chunk. [`transform_chunk`]
//! composes wok-scene's slicer with wok-mesh's terrain generation to turn one authored chunk into a
//! [`ChunkRuntime`], and [`ChunkStore`] tracks per-chunk lifecycle state and owns the runtimes of
//! loaded chunks, keyed by wok-scene's `ChunkCoord`. After the transform the authored chunk is no
//! longer referenced; `transform_chunk` consumes it, so the rule is enforced by move.
//!
//! Part 1 (this revision) is the synchronous lifecycle:
//! - [`ChunkState`]: the load/unload state machine, illegal transitions as errors. Part 1 drives it
//!   synchronously, passing `Loading` / `Unloading` through atomically inside [`ChunkStore::load`]
//!   and [`ChunkStore::release`]; the machine is defined in full now so part 2's background worker
//!   extends the store rather than retrofitting states. See `crate::state`.
//! - [`ChunkRuntime`] / [`transform_chunk`]: the slice arrays (visible, hitbox, trigger; chunk-local
//!   transforms), the terrain mesh when the chunk has a heightmap, the heightmap itself (moved to
//!   runtime, physics samples it), and the referenced asset names, unresolved. See `crate::runtime`.
//! - [`ChunkStore`]: load, release, get, and iteration over loaded chunks in deterministic
//!   coordinate order. See `crate::store`.
//!
//! Part 2 adds the streaming policy (the desired-loaded-set computation from scene topology plus
//! player position, the eagerness modes, Vista enforcement, eviction, the chunk budget) and the
//! background worker. Nothing in part 1 assumes a specific policy.
//!
//! Asset references stay names. A [`ChunkRuntime`]'s `referenced_assets` is the per-chunk seed the
//! future content scan reads to build the missing-assets list; no file loading happens here.
//!
//! Determinism (canon contract): `transform_chunk` produces identical arrays from identical inputs.
//! No threads, no clocks, no RNG anywhere in this part.
//!
//! No tracing: part 1's lifecycle is synchronous, so every load and release is the caller's own call
//! and an event would tell the call site nothing it does not already know (the wok-light precedent:
//! no events without something genuinely worth tracing). The part 2 worker, where lifecycle becomes
//! asynchronous and invisible to the caller, is where chunk-lifecycle debug/info events earn their
//! place, and where the tracing dependency question belongs.

#[cfg(test)]
pub(crate) mod fixture;
pub mod runtime;
pub mod state;
pub mod store;

pub use runtime::{ChunkRuntime, TransformError, transform_chunk};
pub use state::{ChunkState, TransitionError};
pub use store::{ChunkStore, StoreError};
