//! Searches print orientations of a part that stands on a flat cut (like the
//! femur sample) for the one that needs the least support on the outside.
//!
//! ```text
//! cargo run --release --example orient_search -- models/femur_proximal_variant.stl
//! ```
//!
//! For each tilt of the up direction, the flat base would be cut again
//! square to the new up direction, so everything within the height of that
//! re-cut is ignored. Above it, two things need support: faces that look down
//! more steeply than `MAX_OVERHANG` from vertical, and points that are lower
//! than all their neighbours (each starts a region in mid-air).

use anyhow::Result;
use bone_infill::analyze::{AnalyzeParams, analyze};
use bone_infill::io;
use bone_infill::mesh::TriMesh;
use bone_infill::section::{SectionParams, section};
use std::path::PathBuf;

const MAX_OVERHANG_DEG: f64 = 45.0;

fn main() -> Result<()> {
    let path: PathBuf = std::env::args().nth(1).unwrap_or_else(|| "models/femur_proximal_variant.stl".into()).into();
    let mesh = io::read_mesh(&path)?;
    let (lo, _) = mesh.bounds();

    // The flat base: its centre and radius tell how much a tilt has to cut.
    let base: Vec<[f64; 3]> = mesh
        .vertices
        .iter()
        .filter(|v| (v[2] as f64) < lo[2] + 0.05)
        .map(|v| [v[0] as f64, v[1] as f64, v[2] as f64])
        .collect();
    let centre = [0, 1, 2].map(|a| base.iter().map(|v| v[a]).sum::<f64>() / base.len() as f64);
    let radius =
        base.iter().map(|v| ((v[0] - centre[0]).powi(2) + (v[1] - centre[1]).powi(2)).sqrt()).fold(0.0, f64::max);
    println!("base: {} vertices, radius {radius:.1} mm", base.len());

    let neighbours = neighbours(&mesh);
    let mut rows = Vec::new();
    for tilt in [0.0f64, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0] {
        for heading in (0..12).map(|i| i as f64 * 30.0) {
            if tilt == 0.0 && heading > 0.0 {
                continue;
            }
            let (t, h) = (tilt.to_radians(), heading.to_radians());
            let up = [t.sin() * h.cos(), t.sin() * h.sin(), t.cos()];
            let (area, minima, cut) = cost(&mesh, &neighbours, up, radius * 2.0 * t.sin() + 1.0);
            rows.push((tilt, heading, area, minima, cut));
        }
    }
    rows.sort_by(|a, b| (a.3, a.2).partial_cmp(&(b.3, b.2)).unwrap());
    println!("tilt  heading  overhang>45 (mm2)  starts in mid-air  re-cut (mm)");
    for (tilt, heading, area, minima, cut) in rows.iter().take(12) {
        println!("{tilt:4.0}  {heading:7.0}  {area:17.0}  {minima:17}  {cut:11.1}");
    }
    let upright = rows.iter().find(|r| r.0 == 0.0).unwrap();
    println!("upright: {:.0} mm2, {} starts in mid-air", upright.2, upright.3);

    // The best few, cut for real and checked like a slicer would.
    println!("\nverified (base re-cut, then the printability check at 45 deg / 4 mm bridges):");
    let mut verify: Vec<_> = rows.iter().take(6).copied().collect();
    verify.push(*upright);
    for (tilt, heading, _, _, cut) in verify {
        let (t, h) = (tilt.to_radians(), heading.to_radians());
        let up = [t.sin() * h.cos(), t.sin() * h.sin(), t.cos()];
        let heights: Vec<f64> = mesh.vertices.iter().map(|v| (0..3).map(|a| v[a] as f64 * up[a]).sum()).collect();
        let floor = heights.iter().cloned().fold(f64::INFINITY, f64::min) + cut;
        let top = heights.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let thickness = top - floor + 2.0;
        let mid = floor + 0.5 * thickness;
        // Any direction across `up` for the slab's own Y.
        let side = if up[2].abs() < 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
        let part = section(
            &mesh,
            &SectionParams {
                through: up.map(|u| u * mid),
                normal: up,
                up: side,
                thickness,
                voxel: 0.25,
                ..SectionParams::default()
            },
        )?;
        let r = analyze(&part, None, &AnalyzeParams::default());
        let (lo, hi) = part.bounds();
        println!(
            "  tilt {tilt:2.0} heading {heading:3.0}: {:6.1} mm2 unsupported, {} islands ({:.1} mm2), height {:.0} mm; down-facing 45-60/60-75/75-90: {:.0}/{:.0}/{:.0} mm2; worst layers {:?}",
            r.unsupported_interior_area,
            r.interior_islands,
            r.interior_island_area,
            hi[2] - lo[2],
            r.down_facing_area_by_angle[2],
            r.down_facing_area_by_angle[3],
            r.down_facing_area_by_angle[4],
            r.worst_layers.iter().take(3).map(|l| format!("{:.0} mm: {:.0} mm2", l.0, l.1)).collect::<Vec<_>>()
        );
    }
    Ok(())
}

fn neighbours(mesh: &TriMesh) -> Vec<Vec<u32>> {
    let mut n: Vec<Vec<u32>> = vec![Vec::new(); mesh.vertices.len()];
    for t in &mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            n[a as usize].push(b);
            n[b as usize].push(a);
        }
    }
    for l in &mut n {
        l.sort_unstable();
        l.dedup();
    }
    n
}

/// Down-facing area steeper than the limit and number of local minima, both
/// above the re-cut zone; also returns the height of that zone.
fn cost(mesh: &TriMesh, neighbours: &[Vec<u32>], up: [f64; 3], cut: f64) -> (f64, usize, f64) {
    let height = |v: &[f32; 3]| v[0] as f64 * up[0] + v[1] as f64 * up[1] + v[2] as f64 * up[2];
    let heights: Vec<f64> = mesh.vertices.iter().map(height).collect();
    let floor = heights.iter().cloned().fold(f64::INFINITY, f64::min) + cut;
    let limit = -MAX_OVERHANG_DEG.to_radians().sin();
    let mut area = 0.0;
    for t in &mesh.triangles {
        let [a, b, c] = t.map(|i| mesh.vertices[i as usize].map(|x| x as f64));
        if [t[0], t[1], t[2]].iter().all(|&i| heights[i as usize] < floor + 0.5) {
            continue;
        }
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len == 0.0 {
            continue;
        }
        let nz = (n[0] * up[0] + n[1] * up[1] + n[2] * up[2]) / len;
        if nz < limit {
            area += 0.5 * len;
        }
    }
    let minima = (0..mesh.vertices.len())
        .filter(|&i| heights[i] > floor + 0.5 && neighbours[i].iter().all(|&j| heights[j as usize] > heights[i]))
        .count();
    (area, minima, cut)
}
