//! Generates an original demo model: a slice through the upper end of a femur.
//!
//! ```text
//! cargo run --release --example femur
//! ```
//!
//! writes `models/femur_section.stl`, a flat slab whose outline is the frontal
//! section of a proximal femur (shaft, greater and lesser trochanter, neck and
//! head). It is meant to be filled with `--open=+z,-z`, so the trabecular
//! lattice shows on both faces, like the plate in an anatomy book.
//!
//! The outline is not traced from a scan: it is a smooth union of a few
//! circles and tapered capsules with roughly anatomical proportions (a 46 mm
//! head, a 128 degree neck-shaft angle). Coordinates are millimetres, X
//! towards the body's midline, Y up the bone, Z through the slab.

use bone_infill::grid::Grid;
use bone_infill::{io, isosurface, simplify};
use rayon::prelude::*;
use std::path::Path;

/// Slab thickness (mm).
const THICKNESS: f32 = 18.0;
/// Sampling pitch of the outline (mm).
const PITCH: f64 = 0.25;

/// Smooth minimum: a union with a fillet of about `k` millimetres.
fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

fn circle(p: [f32; 2], c: [f32; 2], r: f32) -> f32 {
    (p[0] - c[0]).hypot(p[1] - c[1]) - r
}

/// Distance to a capsule from `a` (radius `ra`) to `b` (radius `rb`).
fn tapered_capsule(p: [f32; 2], a: [f32; 2], b: [f32; 2], ra: f32, rb: f32) -> f32 {
    let axis = [b[0] - a[0], b[1] - a[1]];
    let h = axis[0].hypot(axis[1]);
    let u = [axis[0] / h, axis[1] / h];
    let rel = [p[0] - a[0], p[1] - a[1]];
    // Local frame: y along the axis, x across it (mirrored).
    let q = [(rel[0] * u[1] - rel[1] * u[0]).abs(), rel[0] * u[0] + rel[1] * u[1]];
    let slope = (ra - rb) / h;
    let c = (1.0 - slope * slope).sqrt();
    let k = -slope * q[0] + c * q[1];
    if k < 0.0 {
        q[0].hypot(q[1]) - ra
    } else if k > c * h {
        q[0].hypot(q[1] - h) - rb
    } else {
        c * q[0] + slope * q[1] - ra
    }
}

/// Signed distance to the outline (negative inside).
fn outline(p: [f32; 2]) -> f32 {
    // Shaft, widening towards the top.
    let mut d = tapered_capsule(p, [0.0, -30.0], [0.0, 76.0], 14.5, 19.0);
    // Greater trochanter: the lateral shoulder and its tip.
    d = smooth_min(d, circle(p, [-9.0, 97.0], 17.0), 10.0);
    d = smooth_min(d, circle(p, [-15.0, 109.0], 10.5), 8.0);
    // Lesser trochanter: a bump on the medial side, below the neck.
    d = smooth_min(d, circle(p, [17.5, 60.0], 7.5), 9.0);
    // Neck, narrowest in the middle, and the head.
    d = smooth_min(d, tapered_capsule(p, [3.0, 87.0], [36.0, 113.0], 15.0, 12.5), 12.0);
    d = smooth_min(d, circle(p, [46.0, 121.0], 23.0), 9.0);
    // The bone is cut square across the shaft.
    d.max(-p[1])
}

fn main() -> anyhow::Result<()> {
    let (lo, hi) = ([-36.0, 0.0, 0.0], [72.0, 146.0, THICKNESS as f64]);
    let grid = Grid::enclosing(lo, hi, PITCH, 4.0 * PITCH);
    let [nx, ny, _] = grid.dims;
    let (xs, ys, zs) = (grid.axis_coords(0), grid.axis_coords(1), grid.axis_coords(2));

    // The outline is the same in every layer: evaluate it once.
    let section: Vec<f32> = (0..nx * ny).into_par_iter().map(|i| -outline([xs[i % nx], ys[i / nx]])).collect();
    let mut field = vec![0f32; grid.len()];
    field.par_chunks_mut(nx * ny).enumerate().for_each(|(k, plane)| {
        // Inside the slab: between z = 0 and z = THICKNESS.
        let slab = zs[k].min(THICKNESS - zs[k]);
        for (value, &inside) in plane.iter_mut().zip(&section) {
            *value = inside.min(slab);
        }
    });

    let mesh = simplify::simplify(&isosurface::extract(&field, &grid), 0.01);
    let report = mesh.edge_report();
    anyhow::ensure!(report.is_watertight(), "generated mesh is not closed: {report:?}");
    let (lo, hi) = mesh.bounds();
    let path = Path::new("models").join("femur_section.stl");
    io::write_mesh(&path, &mesh)?;
    println!(
        "{}: {:.1} x {:.1} x {:.1} mm, {:.1} cm3, {} triangles",
        path.display(),
        hi[0] - lo[0],
        hi[1] - lo[1],
        hi[2] - lo[2],
        mesh.signed_volume() / 1000.0,
        mesh.triangles.len()
    );
    Ok(())
}
