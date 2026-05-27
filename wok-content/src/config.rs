//! `ContentConfig` - tunables for streaming, primitive tessellation, terrain palette, etc.
//! Defaults are the engine's contract (plan section 9.9); workspace integration tests
//! exercise the defaults. Deviating changes engine behavior in ways test suites do not
//! catch.
//!
//! Phase A populates the primitive-tessellation and terrain-palette knobs. Streaming knobs
//! (max_loaded_chunks, hysteresis_factor, hysteresis_ticks, vista_multiplier,
//! priority_queue_capacity) carry their default values but are unused until Phase C.

use std::collections::BTreeMap;

/// Tunables for the engine's content layer. Most callers use `ContentConfig::default()`;
/// knobs exist for testing and unusual scenes.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentConfig {
    pub max_loaded_chunks: usize,
    pub hysteresis_factor: f32,
    pub hysteresis_ticks: u64,
    pub vista_multiplier: f32,
    pub ellipsoid_subdivisions: u32,
    pub cylinder_segments: u32,
    pub priority_queue_capacity: usize,
    pub terrain_palette: SurfaceTagPalette,
}

impl Default for ContentConfig {
    fn default() -> Self {
        ContentConfig {
            max_loaded_chunks: 32,
            hysteresis_factor: 1.25,
            hysteresis_ticks: 60,
            vista_multiplier: 1.5,
            ellipsoid_subdivisions: 16,
            cylinder_segments: 24,
            priority_queue_capacity: 64,
            terrain_palette: SurfaceTagPalette::default(),
        }
    }
}

/// `SurfaceTag` -> RGB color, with a fallback for tags that have no entry. The terrain mesh
/// generator looks up each cell's surface tag via `wok_scene::surface_at` (which returns a
/// `&str`) and selects a color from this palette. Unknown tags resolve to `fallback`
/// rather than panicking; the empty-string tag (`""`) is the conventional "untagged" value
/// and maps to whatever the palette assigns it (default: a neutral gray).
///
/// `BTreeMap` over `HashMap` so iteration and serialization order is deterministic (plan
/// section 9.9 references "deterministic on save" as the relevant property for palette
/// reuse across saves).
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceTagPalette {
    pub by_tag: BTreeMap<String, [f32; 3]>,
    pub fallback: [f32; 3],
}

impl Default for SurfaceTagPalette {
    fn default() -> Self {
        let mut by_tag = BTreeMap::new();
        // Defaults are placeholder colors keyed to a small set of conventional surface
        // names: grass / stone / dirt / sand / wood. Plan section 9.9 leaves the concrete
        // palette shape to Phase A; this picks tones that read as distinct categories in a
        // gray-tinted preview render but are not authored canon. Authors override via the
        // ContentConfig field.
        by_tag.insert(String::new(), [0.55, 0.55, 0.55]);
        by_tag.insert("grass".to_string(), [0.32, 0.58, 0.28]);
        by_tag.insert("stone".to_string(), [0.52, 0.52, 0.52]);
        by_tag.insert("dirt".to_string(), [0.45, 0.32, 0.22]);
        by_tag.insert("sand".to_string(), [0.82, 0.74, 0.55]);
        by_tag.insert("wood".to_string(), [0.50, 0.36, 0.22]);
        SurfaceTagPalette {
            by_tag,
            fallback: [0.80, 0.20, 0.80], // magenta - the conventional "missing" placeholder
        }
    }
}

impl SurfaceTagPalette {
    /// Look up the color for a surface tag. Returns the fallback color for unknown tags.
    pub fn color(&self, tag: &str) -> [f32; 3] {
        self.by_tag.get(tag).copied().unwrap_or(self.fallback)
    }
}
