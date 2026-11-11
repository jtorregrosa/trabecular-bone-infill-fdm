//! Narrow-band signed distance to a triangle mesh, exact to the triangles.
//!
//! Used when faces are opened: the lattice is then cut by the model surface
//! itself, and a distance taken from voxel occupancy would leave stair steps.

use crate::grid::{Grid, flag};
use crate::mesh::{TriMesh, to_f64};
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// Signed distance (mm, positive inside) clamped to `±band`. `flags` supplies
/// the sign through its `INSIDE` bit.
pub fn narrow_band(mesh: &TriMesh, grid: &Grid, flags: &[u8], band: f32) -> Vec<f32> {
    let [nx, ny, nz] = grid.dims;
    // Non-negative f32 bit patterns order like the floats, so an atomic
    // integer minimum is an atomic float minimum.
    let cells: Vec<AtomicU32> = (0..grid.len()).map(|_| AtomicU32::new(band.to_bits())).collect();
    let band64 = band as f64;
    let (o, h) = (grid.origin, grid.spacing);

    mesh.triangles.par_iter().for_each(|t| {
        let a = to_f64(mesh.vertices[t[0] as usize]);
        let b = to_f64(mesh.vertices[t[1] as usize]);
        let c = to_f64(mesh.vertices[t[2] as usize]);
        let n = cross(sub(b, a), sub(c, a));
        let len = dot(n, n).sqrt();
        if len == 0.0 {
            return;
        }
        let n = [n[0] / len, n[1] / len, n[2] / len];
        let range = |axis: usize, count: usize| {
            let lo = a[axis].min(b[axis]).min(c[axis]) - band64;
            let hi = a[axis].max(b[axis]).max(c[axis]) + band64;
            let first = ((lo - o[axis]) / h[axis]).ceil().max(0.0) as usize;
            let last = ((hi - o[axis]) / h[axis]).floor();
            let end = if last < 0.0 { 0 } else { (last as usize + 1).min(count) };
            (first, end)
        };
        let (i0, i1) = range(0, nx);
        let (j0, j1) = range(1, ny);
        let (k0, k1) = range(2, nz);
        for k in k0..k1 {
            let z = o[2] + k as f64 * h[2];
            for j in j0..j1 {
                let y = o[1] + j as f64 * h[1];
                // Restrict x to the slab |plane distance| <= band.
                let rest = n[1] * (y - a[1]) + n[2] * (z - a[2]);
                let (mut ia, mut ib) = (i0, i1);
                if n[0].abs() > 1e-9 {
                    let x1 = a[0] + (-band64 - rest) / n[0];
                    let x2 = a[0] + (band64 - rest) / n[0];
                    let (xl, xh) = if x1 < x2 { (x1, x2) } else { (x2, x1) };
                    ia = ia.max(((xl - o[0]) / h[0]).ceil().max(0.0) as usize);
                    let top = ((xh - o[0]) / h[0]).floor();
                    ib = if top < 0.0 { 0 } else { ib.min(top as usize + 1) };
                } else if rest.abs() > band64 {
                    continue;
                }
                for i in ia..ib {
                    let p = [o[0] + i as f64 * h[0], y, z];
                    let d = point_triangle_distance(p, a, b, c) as f32;
                    if d < band {
                        cells[(k * ny + j) * nx + i].fetch_min(d.to_bits(), Ordering::Relaxed);
                    }
                }
            }
        }
    });

    cells
        .into_par_iter()
        .zip(flags.par_iter())
        .map(|(cell, &f)| {
            let d = f32::from_bits(cell.into_inner());
            if f & flag::INSIDE != 0 { d } else { -d }
        })
        .collect()
}

#[inline]
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Distance from `p` to triangle `abc` (Ericson, *Real-Time Collision
/// Detection*, closest point by Voronoi regions).
pub fn point_triangle_distance(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> f64 {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    let closest = 'found: {
        if d1 <= 0.0 && d2 <= 0.0 {
            break 'found a;
        }
        let bp = sub(p, b);
        let d3 = dot(ab, bp);
        let d4 = dot(ac, bp);
        if d3 >= 0.0 && d4 <= d3 {
            break 'found b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            let v = d1 / (d1 - d3);
            break 'found [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]];
        }
        let cp = sub(p, c);
        let d5 = dot(ab, cp);
        let d6 = dot(ac, cp);
        if d6 >= 0.0 && d5 <= d6 {
            break 'found c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            let w = d2 / (d2 - d6);
            break 'found [a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]];
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
            break 'found [b[0] + w * (c[0] - b[0]), b[1] + w * (c[1] - b[1]), b[2] + w * (c[2] - b[2])];
        }
        let denom = 1.0 / (va + vb + vc);
        let v = vb * denom;
        let w = vc * denom;
        [a[0] + ab[0] * v + ac[0] * w, a[1] + ab[1] * v + ac[1] * w, a[2] + ab[2] * v + ac[2] * w]
    };
    let d = sub(p, closest);
    dot(d, d).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;
    use crate::voxelize::voxelize;

    #[test]
    fn point_triangle_regions() {
        let (a, b, c) = ([0.0, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 2.0, 0.0]);
        assert!((point_triangle_distance([0.5, 0.5, 3.0], a, b, c) - 3.0).abs() < 1e-12); // face
        assert!((point_triangle_distance([-1.0, -1.0, 0.0], a, b, c) - 2f64.sqrt()).abs() < 1e-12); // vertex
        assert!((point_triangle_distance([1.0, -2.0, 0.0], a, b, c) - 2.0).abs() < 1e-12); // edge
        assert!((point_triangle_distance([2.0, 2.0, 0.0], a, b, c) - 2f64.sqrt()).abs() < 1e-12); // hypotenuse
    }

    #[test]
    fn box_band_matches_analytic_distance() {
        let mesh = box_mesh([0.0; 3], [6.0, 5.0, 4.0]);
        let grid = Grid::enclosing([0.0; 3], [6.0, 5.0, 4.0], 0.25, 1.1);
        let mut flags = voxelize(&mesh, &grid);
        flags.iter_mut().for_each(|f| *f = if *f != 0 { flag::INSIDE } else { 0 });
        let band = 0.8f32;
        let sdf = narrow_band(&mesh, &grid, &flags, band);
        let [nx, ny, nz] = grid.dims;
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let p = [grid.x(i), grid.y(j), grid.z(k)];
                    let hi = [6.0, 5.0, 4.0];
                    // Analytic signed distance to the box (positive inside).
                    let q = [0, 1, 2].map(|a| (p[a] - hi[a] / 2.0).abs() - hi[a] / 2.0);
                    let outside = (q[0].max(0.0).powi(2) + q[1].max(0.0).powi(2) + q[2].max(0.0).powi(2)).sqrt();
                    let exact = -(outside + q[0].max(q[1]).max(q[2]).min(0.0));
                    let expect = exact.clamp(-band as f64, band as f64) as f32;
                    let got = sdf[grid.index(i, j, k)];
                    assert!((got - expect).abs() < 1e-4, "at {p:?}: {got} vs {expect}");
                }
            }
        }
    }
}
