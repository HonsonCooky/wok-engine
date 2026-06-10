//! Floating-at-rest diagnostics: does the rendered terrain agree with the sampled terrain?
//!
//! The play-test hypothesis is a horizontal convention mismatch between `Heightmap::height_at`
//! (what physics rests on) and `wok_mesh::terrain_mesh` (what the eye sees): a fencepost or
//! half-cell offset would be invisible on flat ground and scale with slope, exactly the observed
//! pattern. These tests measure the disagreement directly, headless: [`mesh_height_at`] replicates
//! the mesh's own triangulation (the same cell split and planar interpolation the GPU rasterizes)
//! and compares it against the sampler over grids that include cell interiors.
//!
//! Findings, pinned by the tests below:
//!
//! - On a constant ramp the two agree to float roundoff at every probed point, interiors included.
//!   A horizontal offset of `e` cells would show up as `e * slope` metres everywhere on a ramp; the
//!   measured error bounds any such offset to under a tenth of a millimetre of height, i.e. zero.
//! - On curved (sample-style sine) terrain the disagreement is exactly the triangle-vs-bilinear
//!   residue: bounded by a quarter of the per-cell twist (the cross second difference), under a
//!   centimetre, zero at every vertex, sign following curvature rather than slope. No systematic
//!   slope-correlated bias - so the mesh, the sampler, and the origin composition all agree, and
//!   the visible float must come from the rest/placement conventions themselves (the five-sample
//!   footprint max), not from a sampling mismatch.

// Exact float comparison is intended throughout: probe coordinates are exact multiples of 0.25
// and vertex agreement is a bitwise claim about identical inputs.
#![allow(clippy::float_cmp)]

use wok_mesh::MeshCpu;
use wok_scene::{CHUNK_GRID_DIM, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

/// The rendered terrain height at chunk-local `(x, z)`: planar interpolation over the triangle the
/// point falls in, replicating `terrain_mesh`'s triangulation (one vertex per integer sample, each
/// cell split along the `(x, z+1)`-`(x+1, z)` diagonal). This is what rasterizing the mesh shows at
/// that point, computed from the mesh's own vertices.
fn mesh_height_at(mesh: &MeshCpu, x: f32, z: f32) -> f32 {
    let cx = (x.floor() as usize).min(CHUNK_GRID_DIM - 2);
    let cz = (z.floor() as usize).min(CHUNK_GRID_DIM - 2);
    let (fx, fz) = (x - cx as f32, z - cz as f32);
    let h = |ix: usize, iz: usize| mesh.vertices[iz * CHUNK_GRID_DIM + ix].position.y;
    let (h00, h10, h01, h11) = (h(cx, cz), h(cx + 1, cz), h(cx, cz + 1), h(cx + 1, cz + 1));
    if fx + fz <= 1.0 {
        // Triangle (a, c, b): the (x, z), (x, z+1), (x+1, z) corner.
        h00 + (h10 - h00) * fx + (h01 - h00) * fz
    } else {
        // Triangle (b, c, d): the far corner across the diagonal.
        h11 + (h01 - h11) * (1.0 - fx) + (h10 - h11) * (1.0 - fz)
    }
}

fn heightmap_from(f: impl Fn(usize, usize) -> u16) -> Heightmap {
    let heights = (0..CHUNK_GRID_LEN).map(|i| f(i % CHUNK_GRID_DIM, i / CHUNK_GRID_DIM)).collect();
    Heightmap::new(heights, vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap()
}

/// A ramp rising along +x by `delta` raw units per cell: constant slope, zero curvature.
fn ramp(delta: u16) -> Heightmap {
    heightmap_from(|x, _| x as u16 * delta)
}

/// Sample-content-style rolling hills: summed sines, the curved terrain the float was seen on.
fn hills() -> Heightmap {
    heightmap_from(|x, z| {
        let (xf, zf) = (x as f32, z as f32);
        let h = 4.0 * (xf * 0.06).sin() * (zf * 0.05).cos()
            + 1.8 * ((xf + zf) * 0.045).sin()
            + 0.7 * (xf * 0.15).sin() * (zf * 0.13).cos();
        Heightmap::meters_to_raw(h)
    })
}

/// Probe a grid of points (cell interiors included: fractions 0.25, 0.5, 0.75 as well as the
/// vertices) and fold each sampler-vs-mesh error through `visit(x, z, error)`.
fn probe(terrain: &Heightmap, mesh: &MeshCpu, mut visit: impl FnMut(f32, f32, f32)) {
    for zi in 0..(CHUNK_GRID_DIM - 1) * 4 {
        for xi in 0..(CHUNK_GRID_DIM - 1) * 4 {
            let (x, z) = (xi as f32 * 0.25, zi as f32 * 0.25);
            visit(x, z, mesh_height_at(mesh, x, z) - terrain.height_at(x, z));
        }
    }
}

#[test]
fn on_a_ramp_the_mesh_and_the_sampler_agree_everywhere() {
    // Constant slope, zero curvature: bilinear and triangle interpolation are both exact, so ANY
    // disagreement here is a convention offset. A half-cell horizontal offset on this ramp
    // (slope ~0.098 m/m) would read ~49mm at every probe; the bound below is a thousand times
    // tighter, so the offset is zero.
    let terrain = ramp(100);
    let mesh = wok_mesh::terrain_mesh(&terrain);
    let mut max_abs = 0.0f32;
    probe(&terrain, &mesh, |x, z, err| {
        assert!(err.abs() < 5e-5, "mesh and sampler disagree by {err} at ({x}, {z})");
        max_abs = max_abs.max(err.abs());
    });
    assert!(max_abs < 5e-5, "max disagreement {max_abs} should be float roundoff only");
    println!("ramp: max |mesh - sampler| = {max_abs} m");
}

#[test]
fn on_curved_terrain_the_disagreement_is_the_triangle_residue_not_a_bias() {
    // With curvature, the drawn triangles and the bilinear sampler legitimately differ inside a
    // cell by at most a quarter of that cell's twist (the cross second difference), and by nothing
    // at the vertices. Pinning both bounds shows the error tracks curvature, not slope: a
    // horizontal convention offset would instead grow with the gradient and not vanish at
    // vertices. The mean over the symmetric probe grid stays near zero (no systematic bias).
    let terrain = hills();
    let mesh = wok_mesh::terrain_mesh(&terrain);

    // The worst per-cell twist over the whole grid, from the heightmap's own samples.
    let h = |x: f32, z: f32| terrain.height_at(x, z);
    let mut max_twist = 0.0f32;
    for cz in 0..CHUNK_GRID_DIM - 1 {
        for cx in 0..CHUNK_GRID_DIM - 1 {
            let (x, z) = (cx as f32, cz as f32);
            let twist = h(x + 1.0, z + 1.0) + h(x, z) - h(x + 1.0, z) - h(x, z + 1.0);
            max_twist = max_twist.max(twist.abs());
        }
    }

    let (mut sum, mut count, mut max_abs) = (0.0f64, 0u32, 0.0f32);
    probe(&terrain, &mesh, |x, z, err| {
        assert!(
            err.abs() <= max_twist * 0.25 + 1e-5,
            "error {err} at ({x}, {z}) exceeds the triangle-vs-bilinear bound {}",
            max_twist * 0.25
        );
        let on_vertex = x.fract() == 0.0 && z.fract() == 0.0;
        if on_vertex {
            assert!(err.abs() < 1e-5, "vertices must agree exactly, got {err} at ({x}, {z})");
        }
        sum += f64::from(err);
        count += 1;
        max_abs = max_abs.max(err.abs());
    });

    let mean = (sum / f64::from(count)) as f32;
    assert!(mean.abs() < 1e-3, "systematic bias: mean error {mean}");
    // The whole disagreement on sample-style hills stays under a centimetre: far too small to be
    // the visible float, which is the point of this measurement.
    assert!(max_abs < 0.01, "max disagreement {max_abs} should be sub-centimetre");
    println!("hills: max |mesh - sampler| = {max_abs} m, mean = {mean} m, max cell twist = {max_twist} m");
}
