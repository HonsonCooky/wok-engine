//! Surface-tag color palette helpers. The palette type lives in `config.rs`
//! (`SurfaceTagPalette`); this module exists so the terrain mesh generator can import it
//! through `crate::terrain::palette` and future palette adjustments find an obvious home.

pub use crate::config::SurfaceTagPalette;
