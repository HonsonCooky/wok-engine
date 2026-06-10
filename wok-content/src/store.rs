//! The chunk store: per-chunk lifecycle state plus the runtimes of loaded chunks.
//!
//! Keyed by wok-scene's `ChunkCoord`, the chunk identity the whole engine shares; no new id type.
//! Backed by a `BTreeMap` rather than a `HashMap` for one mechanical reason: iteration over loaded
//! chunks is part of the public surface, and a consumer folding per-chunk results (contacts, render
//! lists) must see a deterministic order for the canon's replay contract to hold end to end.
//! `ChunkCoord` is already `Ord`, chunk counts are dozens not millions, and the ordered map costs
//! nothing extra in dependencies.
//!
//! Every lifecycle change is driven through `ChunkState::transition`, so the machine is the single
//! authority on legality: a double load fails as `Loaded -> Loading`, a stray release as
//! `Unloaded -> Unloading`, both as errors the caller handles. Part 1 is synchronous, so a chunk only
//! rests in `Unloaded` (absent from the map) or `Loaded` (present with its runtime); `load` and
//! `release` pass through `Loading` / `Unloading` atomically. Part 2's background worker is what
//! makes the intermediate states observable, by extending this store, not the machine.

use std::collections::{BTreeMap, HashMap};
use std::hash::BuildHasher;

use wok_scene::{Chunk, ChunkCoord, Heightmap, Prefab, PrefabRef};

use crate::runtime::{ChunkRuntime, TransformError, transform_chunk};
use crate::state::{ChunkState, TransitionError};

/// Failure modes of the store's lifecycle operations: either the requested lifecycle change is
/// illegal for the chunk's current state, or the load's transform itself failed. Both wrapped
/// transparently - the inner errors already say everything, and the store adds no context of its own.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Transition(#[from] TransitionError),

    #[error(transparent)]
    Transform(#[from] TransformError),
}

/// Tracks every chunk's lifecycle state and owns the `ChunkRuntime` of each loaded chunk.
#[derive(Debug, Default)]
pub struct ChunkStore {
    loaded: BTreeMap<ChunkCoord, ChunkRuntime>,
}

impl ChunkStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The lifecycle state of the chunk at `coord`. Part 1 is synchronous, so this is `Loaded` for
    /// chunks in the store and `Unloaded` for everything else; an unknown coordinate is simply a
    /// chunk that was never loaded.
    pub fn state(&self, coord: ChunkCoord) -> ChunkState {
        if self.loaded.contains_key(&coord) { ChunkState::Loaded } else { ChunkState::Unloaded }
    }

    /// Load an authored chunk: transform it and store the runtime, keyed by the chunk's own coord.
    ///
    /// Drives `Unloaded -> Loading -> Loaded`; on transform failure the chunk goes back through
    /// `Loading -> Unloaded` (the machine's failed-load arc) and the store is left untouched.
    /// Loading an already-loaded chunk is the illegal `Loaded -> Loading` arc, surfaced as an error:
    /// reload is an explicit release-then-load, so the caller decides when runtime arrays die.
    pub fn load<S: BuildHasher>(
        &mut self,
        chunk: Chunk,
        heightmap: Option<Heightmap>,
        prefabs: &HashMap<PrefabRef, Prefab, S>,
    ) -> Result<&ChunkRuntime, StoreError> {
        let coord = chunk.coord;
        let loading = self.state(coord).transition(ChunkState::Loading)?;
        let runtime = match transform_chunk(chunk, heightmap, prefabs) {
            Ok(runtime) => {
                loading.transition(ChunkState::Loaded)?;
                runtime
            }
            Err(err) => {
                // Failed load: back to Unloaded. Nothing was stored, so the map is untouched.
                loading.transition(ChunkState::Unloaded)?;
                return Err(err.into());
            }
        };
        Ok(self.loaded.entry(coord).or_insert(runtime))
    }

    /// Release a loaded chunk, dropping its runtime arrays.
    ///
    /// Drives `Loaded -> Unloading -> Unloaded`; releasing a chunk that is not loaded is the illegal
    /// `Unloaded -> Unloading` arc, surfaced as an error.
    pub fn release(&mut self, coord: ChunkCoord) -> Result<(), StoreError> {
        let unloading = self.state(coord).transition(ChunkState::Unloading)?;
        unloading.transition(ChunkState::Unloaded)?;
        self.loaded.remove(&coord);
        Ok(())
    }

    /// The runtime of the chunk at `coord`, if it is loaded.
    pub fn get(&self, coord: ChunkCoord) -> Option<&ChunkRuntime> {
        self.loaded.get(&coord)
    }

    /// Iterate the loaded chunks in coordinate order (deterministic; see the module docs).
    pub fn iter_loaded(&self) -> impl Iterator<Item = (ChunkCoord, &ChunkRuntime)> {
        self.loaded.iter().map(|(coord, runtime)| (*coord, runtime))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{fixture_chunk, flat_heightmap, library, simple_chunk};

    #[test]
    fn state_of_a_never_loaded_chunk_is_unloaded() {
        let store = ChunkStore::new();
        assert_eq!(store.state(ChunkCoord::new(5, -5)), ChunkState::Unloaded);
    }

    #[test]
    fn load_stores_the_runtime_and_reports_loaded() {
        let mut store = ChunkStore::new();
        let prefabs = library();
        let coord = ChunkCoord::new(0, 0);

        let runtime = store.load(fixture_chunk(), Some(flat_heightmap(500)), &prefabs).unwrap();
        assert_eq!(runtime.coord, coord);

        assert_eq!(store.state(coord), ChunkState::Loaded);
        let fetched = store.get(coord).unwrap();
        assert_eq!(fetched.visible.len(), 4);
        assert_eq!(fetched.hitboxes.len(), 3);
        assert_eq!(fetched.triggers.len(), 1);
        assert!(fetched.terrain_mesh.is_some());
    }

    #[test]
    fn loading_an_already_loaded_chunk_is_a_transition_error() {
        let mut store = ChunkStore::new();
        let prefabs = library();
        store.load(fixture_chunk(), None, &prefabs).unwrap();

        match store.load(fixture_chunk(), None, &prefabs).unwrap_err() {
            StoreError::Transition(err) => {
                assert_eq!(err.from, ChunkState::Loaded);
                assert_eq!(err.to, ChunkState::Loading);
            }
            other => panic!("expected a transition error, got {other:?}"),
        }
        // The first load's runtime is still there, untouched.
        assert_eq!(store.state(ChunkCoord::new(0, 0)), ChunkState::Loaded);
    }

    #[test]
    fn a_failed_transform_leaves_the_chunk_unloaded() {
        let mut store = ChunkStore::new();
        let empty = HashMap::new();
        let coord = ChunkCoord::new(2, 2);

        match store.load(simple_chunk(2, 2, 1), None, &empty).unwrap_err() {
            StoreError::Transform(TransformError::Slice(_)) => {}
            other => panic!("expected a transform error, got {other:?}"),
        }
        assert_eq!(store.state(coord), ChunkState::Unloaded);
        assert!(store.get(coord).is_none());
        assert_eq!(store.iter_loaded().count(), 0);
    }

    #[test]
    fn release_drops_the_runtime() {
        let mut store = ChunkStore::new();
        let coord = ChunkCoord::new(0, 0);
        store.load(fixture_chunk(), Some(flat_heightmap(100)), &library()).unwrap();

        store.release(coord).unwrap();
        assert_eq!(store.state(coord), ChunkState::Unloaded);
        assert!(store.get(coord).is_none());
        assert_eq!(store.iter_loaded().count(), 0);
    }

    #[test]
    fn releasing_an_unloaded_chunk_is_a_transition_error() {
        let mut store = ChunkStore::new();
        match store.release(ChunkCoord::new(7, 7)).unwrap_err() {
            StoreError::Transition(err) => {
                assert_eq!(err.from, ChunkState::Unloaded);
                assert_eq!(err.to, ChunkState::Unloading);
            }
            other => panic!("expected a transition error, got {other:?}"),
        }
    }

    #[test]
    fn a_released_chunk_can_be_loaded_again() {
        let mut store = ChunkStore::new();
        let prefabs = library();
        let coord = ChunkCoord::new(0, 0);

        store.load(fixture_chunk(), None, &prefabs).unwrap();
        store.release(coord).unwrap();
        store.load(fixture_chunk(), None, &prefabs).unwrap();
        assert_eq!(store.state(coord), ChunkState::Loaded);
    }

    #[test]
    fn iteration_is_in_coordinate_order() {
        let mut store = ChunkStore::new();
        let prefabs = library();
        // Loaded out of order; iteration must come back sorted by ChunkCoord's Ord (x, then z).
        for (x, z) in [(3, 0), (-1, 4), (0, 0), (-1, -2)] {
            store.load(simple_chunk(x, z, 1), None, &prefabs).unwrap();
        }
        let coords: Vec<ChunkCoord> = store.iter_loaded().map(|(coord, _)| coord).collect();
        let expected: Vec<ChunkCoord> = [(-1, -2), (-1, 4), (0, 0), (3, 0)]
            .into_iter()
            .map(|(x, z)| ChunkCoord::new(x, z))
            .collect();
        assert_eq!(coords, expected);
    }
}
