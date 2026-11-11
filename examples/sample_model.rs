//! Builds the web app's sample model ("Try a sample") from a full-resolution
//! mesh.
//!
//! ```text
//! cargo run --release --example sample_model                       (models/femur_proximal.stl)
//! cargo run --release --example sample_model -- other_model.stl
//! ```
//!
//! Three steps, each leaving the anatomy as it is:
//!
//! 1. **Smoothing.** Taubin smoothing removes the small bumps a scan leaves on
//!    the surface. It alternates a smoothing step with a slightly stronger
//!    inflating one, so the part does not shrink as with plain averaging. It
//!    runs on a copy simplified to `SMOOTHING_TRIANGLES`, whose edges match
//!    the size of the bumps. Vertices on the flat base stay where they are, so
//!    it still sits flat.
//! 2. **Individual variation.** No two femurs are the same. A smooth random
//!    displacement, about `WARP_RMS` millimetres with features of about
//!    `WARP_WAVELENGTH`, changes proportions and curvatures a little. That is
//!    far longer than the head, the neck or the trochanters, so each of them
//!    keeps its shape (the head stays practically spherical); only how they
//!    sit relative to each other varies, as between two people. The
//!    displacement fades out near the base. `SEED` picks the variant.
//! 3. **Simplification** with the crate's quadric simplifier to at most
//!    `TARGET` triangles, then a check that the mesh is still closed.
//!
//! The result goes to `src/web/ui/sample.stl`, next to the built page (the
//! native server embeds and serves it, `wasm_site` copies it into
//! `wasm/dist/`, and the page fetches it only when asked for), and at the
//! resolution the smoothing works at to `models/<input>_variant.stl`.

use anyhow::{Context, Result, bail};
use bone_infill::analyze::{AnalyzeParams, analyze};
use bone_infill::mesh::TriMesh;
use bone_infill::section::{SectionParams, section};
use bone_infill::{io, simplify};
use rayon::prelude::*;
use std::path::Path;

/// Triangles to aim for: about 1.4 MB, and the surface within a voxel of the
/// smoothed one.
const TARGET: usize = 30_000;
/// Smoothing passes (each a smoothing and an inflating step).
const SMOOTHING_PASSES: usize = 30;
/// Triangles of the mesh the smoothing works on. Smoothing acts on bumps a
/// few edges across, so a coarser mesh (edges a little over 1 mm) is what
/// lets it remove scan bumps of 1 to 3 mm while features of 10 mm and more
/// (the fovea, the trochanters, the neck) stay.
const SMOOTHING_TRIANGLES: usize = 60_000;
/// Typical size of the variation (mm, root mean square).
const WARP_RMS: f64 = 0.6;
/// Typical length over which the variation changes (mm).
const WARP_WAVELENGTH: f64 = 45.0;
/// Height above the base over which the variation fades in (mm).
const BASE_FADE: f64 = 12.0;
/// Which variant.
const SEED: u64 = 2026;
/// Print tilt (degrees) towards the head, the +Y side of the model. Standing
/// straight, the underside of the head starts in mid-air and needs a
/// support; tilted this much, every layer rests on the one below
/// (`examples/orient_search.rs` found it).
const TILT_DEG: f64 = 20.0;

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let input: std::path::PathBuf =
        std::env::args().nth(1).map_or_else(|| root.join("models/femur_proximal.stl"), Into::into);
    let output = root.join("src/web/ui/sample.stl");
    let variant =
        input.with_file_name(format!("{}_variant.stl", input.file_stem().and_then(|s| s.to_str()).unwrap_or("model")));

    let original = io::read_mesh(&input).with_context(|| format!("reading {}", input.display()))?;
    let (lo, hi) = original.bounds();
    let volume = original.signed_volume();
    println!(
        "{}: {} triangles, {:.1} x {:.1} x {:.1} mm, {:.1} cm3",
        input.display(),
        original.triangles.len(),
        hi[0] - lo[0],
        hi[1] - lo[1],
        hi[2] - lo[2],
        volume / 1000.0
    );

    let (mut mesh, _) = simplify_to(&original, SMOOTHING_TRIANGLES)?;
    report_shape("coarsened", &mesh, volume);
    let base_z = lo[2] as f32;
    let pinned: Vec<bool> = mesh.vertices.iter().map(|v| v[2] < base_z + 0.05).collect();
    taubin(&mut mesh, SMOOTHING_PASSES, &pinned);
    let smoothed = mesh.clone();
    report_shape("smoothed", &mesh, volume);

    warp(&mut mesh, lo[2]);
    report("variation", &smoothed, &mesh, volume);
    let mut mesh = tilt(&mesh, lo[2])?;
    report_shape("tilted and re-cut", &mesh, volume);
    let check = analyze(&mesh, None, &AnalyzeParams::default());
    println!(
        "  standing on its new base: {} regions start in mid-air ({:.1} mm2), {:.0} mm2 overhangs past 45 deg",
        check.interior_islands, check.interior_island_area, check.unsupported_interior_area
    );
    check_closed(&mesh)?;
    // The re-cut is meshed from voxels: back to a manageable size.
    mesh = simplify_to(&mesh, SMOOTHING_TRIANGLES)?.0;
    io::write_mesh(&variant, &mesh)?;
    println!("{}: {} triangles", variant.display(), mesh.triangles.len());

    let (simplified, tolerance) = simplify_to(&mesh, TARGET)?;
    check_closed(&simplified)?;
    io::write_mesh(&output, &simplified)?;
    let bytes = std::fs::metadata(&output)?.len();
    println!(
        "{}: {} triangles, {:.0} KB, simplified within {tolerance:.3} mm",
        output.display(),
        simplified.triangles.len(),
        bytes as f64 / 1024.0
    );
    Ok(())
}

/// Tilts the part `TILT_DEG` towards +Y and cuts the base flat again, square
/// to the new vertical, so it stands on the bed with the head leaning out.
/// The cut removes the wedge of shaft that the tilt lifts off the bed.
fn tilt(mesh: &TriMesh, base_z: f64) -> Result<TriMesh> {
    let t = TILT_DEG.to_radians();
    let up = [0.0, t.sin(), t.cos()];
    let base: Vec<[f64; 3]> =
        mesh.vertices.iter().filter(|v| (v[2] as f64) < base_z + 0.05).map(|v| v.map(|x| x as f64)).collect();
    let height = |v: &[f64; 3]| v[0] * up[0] + v[1] * up[1] + v[2] * up[2];
    // Cut just above the highest point of the old base, so the new one is whole.
    let floor = base.iter().map(height).fold(f64::NEG_INFINITY, f64::max) + 0.5;
    let top = mesh.vertices.iter().map(|v| height(&v.map(|x| x as f64))).fold(f64::NEG_INFINITY, f64::max);
    let thickness = top - floor + 2.0;
    section(
        mesh,
        &SectionParams {
            through: up.map(|u| u * (floor + 0.5 * thickness)),
            normal: up,
            // The head stays on the +Y side, so views and sections read as before.
            up: [0.0, 1.0, 0.0],
            thickness,
            voxel: 0.2,
            ..SectionParams::default()
        },
    )
}

/// Taubin's λ|μ smoothing with uniform weights; `pinned` vertices do not move.
fn taubin(mesh: &mut TriMesh, passes: usize, pinned: &[bool]) {
    let n = mesh.vertices.len();
    let mut neighbours: Vec<Vec<u32>> = vec![Vec::new(); n];
    for t in &mesh.triangles {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            neighbours[a as usize].push(b);
            neighbours[b as usize].push(a);
        }
    }
    for list in &mut neighbours {
        list.sort_unstable();
        list.dedup();
    }
    // λ = 0.5 smooths; μ = -0.53 inflates back a little more, which is what
    // keeps the volume.
    for _ in 0..passes {
        for factor in [0.5f32, -0.53] {
            let current = &mesh.vertices;
            let next: Vec<[f32; 3]> = (0..n)
                .into_par_iter()
                .map(|i| {
                    let v = current[i];
                    if pinned[i] || neighbours[i].is_empty() {
                        return v;
                    }
                    let mut c = [0.0f32; 3];
                    for &j in &neighbours[i] {
                        for a in 0..3 {
                            c[a] += current[j as usize][a];
                        }
                    }
                    let k = 1.0 / neighbours[i].len() as f32;
                    [0, 1, 2].map(|a| v[a] + factor * (c[a] * k - v[a]))
                })
                .collect();
            mesh.vertices = next;
        }
    }
}

/// A smooth random displacement field: per axis, a sum of plane waves with
/// random directions, wavelengths around `WARP_WAVELENGTH` and phases.
fn warp(mesh: &mut TriMesh, base_z: f64) {
    const WAVES: usize = 6;
    let mut state = SEED;
    let mut random = move || {
        // splitmix64
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    // [axis][wave] = (wave vector, phase)
    let waves: Vec<Vec<([f64; 3], f64)>> = (0..3)
        .map(|_| {
            (0..WAVES)
                .map(|_| {
                    let z = 2.0 * random() - 1.0;
                    let angle = std::f64::consts::TAU * random();
                    let r = (1.0 - z * z).sqrt();
                    let length = WARP_WAVELENGTH * (0.8 + 0.4 * random());
                    let k = std::f64::consts::TAU / length;
                    ([k * r * angle.cos(), k * r * angle.sin(), k * z], std::f64::consts::TAU * random())
                })
                .collect()
        })
        .collect();
    let amplitude = WARP_RMS * (2.0 / WAVES as f64).sqrt();
    mesh.vertices.par_iter_mut().for_each(|v| {
        let p = [v[0] as f64, v[1] as f64, v[2] as f64];
        let t = ((p[2] - base_z) / BASE_FADE).clamp(0.0, 1.0);
        let fade = t * t * (3.0 - 2.0 * t);
        for a in 0..3 {
            let d: f64 =
                waves[a].iter().map(|(k, phase)| (k[0] * p[0] + k[1] * p[1] + k[2] * p[2] + phase).cos()).sum();
            v[a] = (p[a] + fade * amplitude * d) as f32;
        }
    });
}

/// The mesh simplified with the smallest tolerance (in steps of about 40 %)
/// that brings it to at most `triangles`.
fn simplify_to(mesh: &TriMesh, triangles: usize) -> Result<(TriMesh, f32)> {
    let mut tolerance = 0.01f32;
    loop {
        let candidate = simplify::simplify(mesh, tolerance);
        if candidate.triangles.len() <= triangles {
            return Ok((candidate, tolerance));
        }
        if tolerance > 1.0 {
            bail!("could not reach {triangles} triangles with a tolerance under 1 mm");
        }
        tolerance *= 1.4;
    }
}

fn report_shape(stage: &str, mesh: &TriMesh, volume: f64) {
    println!(
        "  {stage}: {} triangles, volume {:+.2} %",
        mesh.triangles.len(),
        100.0 * (mesh.signed_volume() / volume - 1.0)
    );
}

fn report(stage: &str, original: &TriMesh, mesh: &TriMesh, volume: f64) {
    let moved: Vec<f64> = original
        .vertices
        .par_iter()
        .zip(mesh.vertices.par_iter())
        .map(|(a, b)| (0..3).map(|i| ((a[i] - b[i]) as f64).powi(2)).sum::<f64>().sqrt())
        .collect();
    let max = moved.iter().cloned().fold(0.0, f64::max);
    let rms = (moved.iter().map(|d| d * d).sum::<f64>() / moved.len() as f64).sqrt();
    println!(
        "  {stage}: vertices moved {rms:.2} mm on average (rms), {max:.2} mm at most; volume {:+.2} %",
        100.0 * (mesh.signed_volume() / volume - 1.0)
    );
}

fn check_closed(mesh: &TriMesh) -> Result<()> {
    let edges = mesh.edge_report();
    if edges.boundary + edges.non_manifold + edges.misoriented > 0 {
        bail!("the mesh is not closed: {edges:?}");
    }
    Ok(())
}
