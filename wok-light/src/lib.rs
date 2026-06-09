//! wok-light: the lighting data model.
//!
//! This crate models the lighting environment a scene runs under, and nothing more (yet). It is
//! pure authored data and its JSON serialization, in the same spirit as wok-scene: no GPU, no
//! runtime pool, no clock reads. Higher layers consume these types - wok-render to draw, the
//! editor to author, the still-to-come dynamic light pool to budget point lights.
//!
//! Part 1 (this revision) is the data model only:
//! - `LightState`: the named lighting snapshot a scene's `LightStateRef` points at (sun, ambient
//!   floor, fog, gradient sky, cel parameters). See `crate::state`.
//! - `LightCurve`: keyframed animation over `LightState`, a pure `sample(t)` evaluator with linear
//!   interpolation and an optional loop. See `crate::curve`.
//! - `PointLight`: the data for one dynamic point light; the pool that manages a set of them comes
//!   later. See `crate::point`.
//! - Per-file JSON load and save (`crate::io`), where a light state's name is its file stem,
//!   consistent with wok-scene's name-based references.
//!
//! Deferred to later parts: the offline static-light bake (needs wok-physics raycasts) and the
//! engine-owned dynamic light pool and its budget. No GPU work belongs here at any point.
//!
//! The crate has no internal dependency. It shares no type with wok-scene: a `LightStateRef` is a
//! bare name there, and the name-to-file mapping is the file-stem convention here, so neither side
//! needs the other's types. Colours are linear-RGB `glam::Vec3`, serialized as bare `[r, g, b]`
//! arrays through `crate::serde_vec3` (wok-scene's hand-rolled pattern, not glam's serde feature).

pub mod curve;
pub mod error;
pub mod io;
pub mod point;
pub(crate) mod serde_vec3;
pub mod state;

pub use curve::{Keyframe, LightCurve};
pub use error::{LoadError, SaveError};
pub use io::{load_light_curve, load_light_state, save_light_curve, save_light_state};
pub use point::PointLight;
pub use state::{CelParams, Fog, LightState, SkyGradient, Sun};
