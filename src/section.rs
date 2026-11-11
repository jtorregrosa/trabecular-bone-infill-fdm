//! Cuts a flat slab out of a model, ready to be filled with open faces.
//!
//! The lattice is inside the part, so the pieces that show it best are slices:
//! a slab of constant thickness, printed lying down, with both faces left
//! without skin. This module makes such a slab from any closed mesh and any
//! cutting plane, and lays it flat (the plane's normal becomes Z).
//!
//! The cut is done on a voxel grid against an exact narrow-band distance to
//! the triangles: the outline follows the model to a fraction of a voxel and
//! the two faces are exactly planar.

use crate::grid::{Grid, flag};
use crate::mesh::TriMesh;
use crate::voxelize::voxelize;
use crate::{isosurface, sdf, simplify};
use anyhow::{Result, bail};
use rayon::prelude::*;

#[derive(Clone, Debug)]
pub struct SectionParams {
    /// A point the mid-plane of the slab passes through (model coordinates).
    pub through: [f64; 3],
    /// Normal of the cutting plane. It becomes +Z of the result.
    pub normal: [f64; 3],
    /// Direction that becomes +Y of the result (its part within the plane).
    pub up: [f64; 3],
    /// Slab thickness (mm), after scaling.
    pub thickness: f64,
    /// Keep only what lies within this distance below `through` along `up`
    /// (mm, after scaling). `None` keeps everything.
    pub below: Option<f64>,
    /// Likewise above `through`.
    pub above: Option<f64>,
    /// Uniform scale applied to the model before cutting.
    pub scale: f64,
    /// Sampling pitch (mm).
    pub voxel: f64,
}

impl Default for SectionParams {
    fn default() -> Self {
        Self {
            through: [0.0; 3],
            normal: [0.0, 1.0, 0.0],
            up: [0.0, 0.0, 1.0],
            thickness: 18.0,
            below: None,
            above: None,
            scale: 1.0,
            voxel: 0.25,
        }
    }
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: [f64; 3]) -> Option<[f64; 3]> {
    let len = dot(a, a).sqrt();
    (len > 1e-9 && len.is_finite()).then(|| [a[0] / len, a[1] / len, a[2] / len])
}

/// Returns the slab as a closed mesh resting on `z = 0`, with its lower-left
/// corner at the origin.
pub fn section(mesh: &TriMesh, p: &SectionParams) -> Result<TriMesh> {
    if !(p.thickness > 0.0 && p.scale > 0.0 && p.voxel > 0.0) {
        bail!("thickness, scale and voxel must be positive");
    }
    if p.thickness < 4.0 * p.voxel {
        bail!("the slab ({} mm) is too thin for a {} mm voxel", p.thickness, p.voxel);
    }
    let Some(n) = unit(p.normal) else { bail!("--normal cannot be the zero vector") };
    let along = dot(p.up, n);
    let Some(up) = unit([p.up[0] - along * n[0], p.up[1] - along * n[1], p.up[2] - along * n[2]]) else {
        bail!("--up must not be parallel to --normal");
    };
    let right = cross(up, n);

    // The model in the slab's frame: X right, Y up, Z along the normal.
    let vertices: Vec<[f32; 3]> = mesh
        .vertices
        .par_iter()
        .map(|v| {
            let d = [v[0] as f64 - p.through[0], v[1] as f64 - p.through[1], v[2] as f64 - p.through[2]];
            [(dot(d, right) * p.scale) as f32, (dot(d, up) * p.scale) as f32, (dot(d, n) * p.scale) as f32]
        })
        .collect();
    let placed = TriMesh { vertices, triangles: mesh.triangles.clone() };

    let half = 0.5 * p.thickness;
    let y_lo = p.below.map(|b| -b);
    let y_hi = p.above;
    // Extent of the part of the model that can reach the slab.
    let reach = half + 4.0 * p.voxel;
    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for t in &placed.triangles {
        let tri = t.map(|v| placed.vertices[v as usize]);
        let (zmin, zmax) = tri.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| (a.min(v[2]), b.max(v[2])));
        if zmin as f64 > reach || (zmax as f64) < -reach {
            continue;
        }
        for v in &tri {
            for a in 0..2 {
                lo[a] = lo[a].min(v[a] as f64);
                hi[a] = hi[a].max(v[a] as f64);
            }
        }
    }
    if lo[0] > hi[0] {
        bail!("the cutting plane does not pass through the model");
    }
    if let Some(y) = y_lo {
        lo[1] = lo[1].max(y);
    }
    if let Some(y) = y_hi {
        hi[1] = hi[1].min(y);
    }
    if lo[1] >= hi[1] {
        bail!("nothing is left between --below and --above");
    }

    let grid = Grid::enclosing([lo[0], lo[1], -half], [hi[0], hi[1], half], p.voxel, 4.0 * p.voxel);
    if grid.len() > 400_000_000 {
        bail!("the section would need {} million samples; use a larger --voxel", grid.len() / 1_000_000);
    }
    let mut flags = voxelize(&placed, &grid);
    flags.par_iter_mut().for_each(|f| *f = if *f != 0 { flag::INSIDE } else { 0 });
    let mut field = sdf::narrow_band(&placed, &grid, &flags, 2.5 * p.voxel as f32);
    drop(flags);

    // Intersect with the slab and the optional crop.
    let (ys, zs) = (grid.axis_coords(1), grid.axis_coords(2));
    let [nx, ny, _] = grid.dims;
    let (half, y_lo, y_hi) = (half as f32, y_lo.map(|v| v as f32), y_hi.map(|v| v as f32));
    field.par_chunks_mut(nx * ny).enumerate().for_each(|(k, plane)| {
        let slab = half - zs[k].abs();
        for (j, row) in plane.chunks_mut(nx).enumerate() {
            let mut limit = slab;
            if let Some(y) = y_lo {
                limit = limit.min(ys[j] - y);
            }
            if let Some(y) = y_hi {
                limit = limit.min(y - ys[j]);
            }
            for value in row {
                *value = value.min(limit);
            }
        }
    });

    let mut out = simplify::simplify(&isosurface::extract(&field, &grid), (0.04 * p.voxel) as f32);
    if out.is_empty() {
        bail!("the slab is empty: the plane misses the solid part of the model");
    }
    // A plane can cut a model into several pieces; drop the crumbs.
    let (labels, components) = out.components();
    let largest = components.iter().map(|c| c.volume).fold(0.0, f64::max);
    if components.iter().any(|c| c.volume < 0.02 * largest) {
        out.retain_triangles(|t| components[labels[t] as usize].volume >= 0.02 * largest);
    }
    // Rest on z = 0 with the lower-left corner at the origin.
    let (min, _) = out.bounds();
    for v in &mut out.vertices {
        v[0] -= min[0] as f32;
        v[1] -= min[1] as f32;
        v[2] += half;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    #[test]
    fn slab_of_a_box_has_the_expected_size_and_is_closed() {
        let model = box_mesh([0.0; 3], [30.0, 20.0, 40.0]);
        let p = SectionParams { through: [15.0, 10.0, 20.0], thickness: 6.0, voxel: 0.5, ..SectionParams::default() };
        let slab = section(&model, &p).unwrap();
        assert!(slab.edge_report().is_watertight());
        let (lo, hi) = slab.bounds();
        // Cut across Y: X stays X, the model's Z becomes the slab's Y.
        for (a, expect) in [(0, 30.0), (1, 40.0), (2, 6.0)] {
            assert!(lo[a].abs() < 0.05, "axis {a} starts at {}", lo[a]);
            assert!((hi[a] - expect).abs() < 0.3, "axis {a}: {} vs {expect}", hi[a]);
        }
        assert!((slab.signed_volume() - 30.0 * 40.0 * 6.0).abs() / 7200.0 < 0.02);
    }

    #[test]
    fn crop_scale_and_oblique_planes() {
        let model = box_mesh([0.0; 3], [30.0, 20.0, 40.0]);
        let base =
            SectionParams { through: [15.0, 10.0, 20.0], thickness: 6.0, voxel: 0.5, ..SectionParams::default() };
        // Keep 5 mm below and 10 mm above the centre.
        let cropped = section(&model, &SectionParams { below: Some(5.0), above: Some(10.0), ..base.clone() }).unwrap();
        let (_, hi) = cropped.bounds();
        assert!((hi[1] - 15.0).abs() < 0.3, "{}", hi[1]);
        // Half size.
        let small = section(&model, &SectionParams { scale: 0.5, ..base.clone() }).unwrap();
        let (_, hi) = small.bounds();
        assert!((hi[0] - 15.0).abs() < 0.3 && (hi[1] - 20.0).abs() < 0.3 && (hi[2] - 6.0).abs() < 0.3);
        // A plane tilted 45 degrees about Z cuts the 30 x 20 footprint diagonally.
        let tilted = section(&model, &SectionParams { normal: [1.0, 1.0, 0.0], ..base.clone() }).unwrap();
        assert!(tilted.edge_report().is_watertight());
        let (_, hi) = tilted.bounds();
        assert!(hi[0] > 27.0 && hi[0] < 34.0, "{}", hi[0]);
    }

    #[test]
    fn impossible_cuts_are_reported() {
        let model = box_mesh([0.0; 3], [10.0; 3]);
        let miss = SectionParams { through: [0.0, 50.0, 0.0], voxel: 0.5, ..SectionParams::default() };
        assert!(section(&model, &miss).is_err());
        let parallel = SectionParams { through: [5.0; 3], up: [0.0, 1.0, 0.0], voxel: 0.5, ..SectionParams::default() };
        assert!(section(&model, &parallel).is_err());
        let thin = SectionParams { through: [5.0; 3], thickness: 0.5, voxel: 0.5, ..SectionParams::default() };
        assert!(section(&model, &thin).is_err());
    }
}
