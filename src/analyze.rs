//! Printability check of a finished mesh, the way a slicer would see it.
//!
//! This deliberately shares nothing with the generator's voxel field: the
//! mesh is re-sliced at the real layer height on a finer XY raster, and each
//! layer is compared with the one below it.
//!
//! A pixel of layer `k` is
//!
//! * **supported** if there is material directly below it;
//! * a **gentle overhang** if material below lies within
//!   `layer_height * tan(max_overhang)` sideways;
//! * a **bridge** if it is part of a band of strands no longer than
//!   `max_bridge`, each resting on supported pixels at both ends (see
//!   [`crate::bridge`]);
//! * **unsupported** otherwise, which is what would need support material.
//!
//! Unsupported area is split into *exterior* (the original model already
//! overhangs there; the slicer's external supports deal with it) and
//! *interior* (introduced by the infill; nothing can be done about it after
//! generation). Interior regions with no anchored pixel at all are counted as
//! *islands*: they would be printed in mid-air.

use crate::bridge::{self, BRIDGED, BridgeScratch, Mode, SUPPORTED, UNSUPPORTED, VOID};
use crate::grid::Grid;
use crate::mesh::{TriMesh, area_normal};
use crate::voxelize::voxelize;
use rayon::prelude::*;
use serde::Serialize;

#[derive(Clone, Debug)]
pub struct AnalyzeParams {
    pub layer_height: f64,
    /// XY raster size (mm).
    pub resolution: f64,
    pub max_overhang_deg: f64,
    pub max_bridge: f64,
    /// Skin thickness of the part (mm). With a reference model, lets the
    /// report say how much of the unsupported area is skin (inner ceilings)
    /// and how much is lattice.
    pub skin: Option<f64>,
    /// Unanchored regions smaller than this (mm²) are counted as specks, not
    /// islands: they are below what a nozzle extrudes.
    pub min_island_area: f64,
    /// Narrowest band of bridge strands accepted as a bridge (mm): two
    /// extrusions by default.
    pub min_bridge_width: f64,
    /// Upper bound on raster pixels per layer; the resolution is coarsened
    /// for parts that would exceed it.
    pub max_pixels_per_layer: usize,
}

impl Default for AnalyzeParams {
    fn default() -> Self {
        Self {
            layer_height: 0.2,
            resolution: 0.1,
            max_overhang_deg: 45.0,
            max_bridge: 4.0,
            skin: None,
            min_island_area: 0.2,
            min_bridge_width: 0.8,
            max_pixels_per_layer: 6_000_000,
        }
    }
}

/// Areas are in mm², summed over all layers (so they scale like the contact
/// area of the support material that would be needed).
#[derive(Clone, Debug, Default, Serialize)]
pub struct PrintabilityReport {
    pub layer_height: f64,
    pub resolution: f64,
    pub max_overhang_deg: f64,
    pub max_bridge: f64,
    pub layers: usize,
    /// Sum of the solid cross-section of every layer.
    pub solid_area: f64,
    /// Material that starts on something less than fully below it but within
    /// the overhang limit.
    pub gentle_overhang_area: f64,
    pub bridge_area: f64,
    /// Longest accepted bridge (mm).
    pub longest_bridge: f64,
    /// Longest two-ended span that was too long to count as a bridge (mm).
    pub longest_rejected_span: f64,
    pub unsupported_interior_area: f64,
    /// Part of `unsupported_interior_area` that belongs to the skin: inner
    /// ceilings and inward-leaning walls. `None` without a reference model
    /// and skin thickness.
    pub unsupported_skin_area: Option<f64>,
    /// The rest: unsupported lattice.
    pub unsupported_lattice_area: Option<f64>,
    pub unsupported_exterior_area: f64,
    /// Interior regions that start in mid-air and are large enough to print.
    pub interior_islands: usize,
    pub interior_island_area: f64,
    /// Mid-air regions too small to be extruded.
    pub interior_specks: usize,
    /// `unsupported_interior_area / solid_area`, in percent.
    pub unsupported_interior_pct: f64,
    /// Down-facing mesh area (mm²) by overhang angle from vertical, bins
    /// `0-30`, `30-45`, `45-60`, `60-75`, `75-90` degrees. Faces lying on the
    /// build plate are excluded; with a reference model, only interior faces
    /// are counted.
    pub down_facing_area_by_angle: [f64; 5],
    /// Layers with the most unsupported interior area: `(z, mm²)`.
    pub worst_layers: Vec<(f64, f64)>,
    /// Where the largest islands are, to look at them in a slicer:
    /// `[x, y, z, area_mm2]`.
    pub island_examples: Vec<[f64; 4]>,
}

/// Within the overhang limit of the layer below: fine to print on, and fine
/// for a bridge to rest on.
const GENTLE: u8 = 4;

#[derive(Default, Clone)]
struct LayerResult {
    solid: usize,
    gentle: usize,
    bridged: usize,
    unsupported_interior: usize,
    unsupported_exterior: usize,
    islands: usize,
    island_pixels: usize,
    specks: usize,
    /// (pixel index of one pixel, pixel count) of each island.
    island_seeds: Vec<(usize, usize)>,
    /// Pixels of the regions in mid-air too small to count as islands, inside
    /// or out. A slicer drops them, so they hold nothing up.
    dust: Vec<u32>,
    /// Interior unsupported pixels, kept only for the skin / lattice split.
    interior_pixels: Vec<u32>,
    /// Per-pixel classification, kept only when a map is requested.
    state: Vec<u8>,
    /// Unsupported pixels the original model also leaves unsupported.
    exterior: Vec<bool>,
    longest_bridge: f32,
    longest_rejected: f32,
}

/// Analyses `mesh`. `reference` is the original model before infill; without
/// it every unsupported pixel is reported as interior.
pub fn analyze(mesh: &TriMesh, reference: Option<&TriMesh>, p: &AnalyzeParams) -> PrintabilityReport {
    let (lo, hi) = mesh.bounds();
    let h = p.layer_height;
    let margin_px = 3.0;
    let mut res = p.resolution;
    let pixels = |res: f64| ((hi[0] - lo[0]) / res + 2.0 * margin_px) * ((hi[1] - lo[1]) / res + 2.0 * margin_px);
    if pixels(res) > p.max_pixels_per_layer as f64 {
        res *= (pixels(res) / p.max_pixels_per_layer as f64).sqrt();
    }
    let nx = ((hi[0] - lo[0]) / res).ceil() as usize + 2 * margin_px as usize + 1;
    let ny = ((hi[1] - lo[1]) / res).ceil() as usize + 2 * margin_px as usize + 1;
    let layers = (((hi[2] - lo[2]) / h) - 1e-6).ceil().max(1.0) as usize;
    let layer = nx * ny;

    let limit = (h * p.max_overhang_deg.to_radians().tan() / res) as f32 + 0.5;
    let offsets = disc_offsets(limit);
    let max_bridge_px = (p.max_bridge / res) as f32;
    let min_band_px = (p.min_bridge_width / res) as f32;
    let min_island_px = (p.min_island_area / (res * res)).ceil() as usize;

    // Depth inside the reference model, to tell skin from lattice.
    let skin_probe = match (reference, p.skin) {
        (Some(r), Some(skin)) => {
            let (rlo, rhi) = r.bounds();
            let size = (0..3).map(|a| rhi[a] - rlo[a]).fold(0.0, f64::max);
            let voxel = (size / 400.0).max(0.2);
            let grid = Grid::enclosing(rlo, rhi, voxel, 3.0 * voxel);
            let occupancy = voxelize(r, &grid);
            let depth = crate::edt::distance_to(grid.dims, |i| occupancy[i] == 0);
            Some((grid, depth, ((skin / voxel) as f32) + 1.0))
        }
        _ => None,
    };
    let collect_pixels = skin_probe.is_some();

    let slab_layers = (64_000_000 / layer).clamp(1, 64);
    let mut results: Vec<LayerResult> = Vec::with_capacity(layers);
    let mut below: Vec<u8> = vec![0; layer];
    let mut below_ref: Vec<u8> = vec![0; layer];
    // Dust of the layer just analysed (see `LayerResult::dust`).
    let mut dust: Vec<u32> = Vec::new();
    let mut k0 = 0usize;
    while k0 < layers {
        let count = slab_layers.min(layers - k0);
        let grid = Grid {
            origin: [lo[0] - margin_px * res, lo[1] - margin_px * res, lo[2] + (k0 as f64 + 0.5) * h],
            spacing: [res, res, h],
            dims: [nx, ny, count],
        };
        let solid = voxelize(mesh, &grid);
        let solid_ref = reference.map(|r| voxelize(r, &grid));

        let below_s = &below;
        let below_ref_s = &below_ref;
        let under_of = |s: usize| -> &[u8] { if s == 0 { below_s } else { &solid[(s - 1) * layer..s * layer] } };
        let run = |s: usize, under: &[u8]| {
            let under_ref: Option<&[u8]> =
                solid_ref.as_ref().map(|r| if s == 0 { &below_ref_s[..] } else { &r[(s - 1) * layer..s * layer] });
            analyze_layer(
                &solid[s * layer..(s + 1) * layer],
                under,
                under_ref,
                k0 + s == 0,
                nx,
                ny,
                limit,
                &offsets,
                max_bridge_px,
                min_band_px,
                min_island_px,
                false,
                collect_pixels,
            )
        };
        let mut slab: Vec<LayerResult> = (0..count).into_par_iter().map(|s| run(s, under_of(s))).collect();

        // Dust holds nothing up: whatever rests only on it starts in mid-air
        // too. Layers above dust are checked again without it, in order, so
        // a strut tip that grows from a speck into a blob is still caught.
        for s in 0..count {
            if !dust.is_empty() {
                let cur = &solid[s * layer..(s + 1) * layer];
                let reach = offsets.iter().copied().chain([(0, 0)]);
                let touched = dust.iter().any(|&p| {
                    let (i, j) = ((p as usize % nx) as i32, (p as usize / nx) as i32);
                    reach.clone().any(|(di, dj)| {
                        let (ii, jj) = (i + di, j + dj);
                        ii >= 0
                            && jj >= 0
                            && ii < nx as i32
                            && jj < ny as i32
                            && cur[jj as usize * nx + ii as usize] != 0
                    })
                });
                if touched {
                    let mut under = under_of(s).to_vec();
                    for &p in &dust {
                        under[p as usize] = 0;
                    }
                    slab[s] = run(s, &under);
                }
            }
            dust = std::mem::take(&mut slab[s].dust);
        }
        results.extend(slab);

        below.copy_from_slice(&solid[(count - 1) * layer..count * layer]);
        if let Some(r) = &solid_ref {
            below_ref.copy_from_slice(&r[(count - 1) * layer..count * layer]);
        }
        k0 += count;
    }

    let px = res * res;
    let mut report = PrintabilityReport {
        layer_height: h,
        resolution: res,
        max_overhang_deg: p.max_overhang_deg,
        max_bridge: p.max_bridge,
        layers,
        ..Default::default()
    };
    let mut per_layer: Vec<(f64, f64)> = Vec::with_capacity(layers);
    let mut examples: Vec<[f64; 4]> = Vec::new();
    if let Some((grid, depth, skin_voxels)) = &skin_probe {
        let skin_pixels: usize = results
            .par_iter()
            .enumerate()
            .map(|(k, r)| {
                let z = lo[2] + (k as f64 + 0.5) * h;
                r.interior_pixels
                    .iter()
                    .filter(|&&pixel| {
                        let pixel = pixel as usize;
                        let x = lo[0] + ((pixel % nx) as f64 - margin_px) * res;
                        let y = lo[1] + ((pixel / nx) as f64 - margin_px) * res;
                        grid.nearest([x, y, z]).is_some_and(|c| depth[grid.index(c[0], c[1], c[2])] <= *skin_voxels)
                    })
                    .count()
            })
            .sum();
        let total: usize = results.iter().map(|r| r.interior_pixels.len()).sum();
        report.unsupported_skin_area = Some(skin_pixels as f64 * px);
        report.unsupported_lattice_area = Some((total - skin_pixels) as f64 * px);
    }
    for (k, r) in results.iter().enumerate() {
        for &(pixel, count) in &r.island_seeds {
            examples.push([
                lo[0] + ((pixel % nx) as f64 - margin_px) * res,
                lo[1] + ((pixel / nx) as f64 - margin_px) * res,
                lo[2] + (k as f64 + 0.5) * h,
                count as f64 * px,
            ]);
        }
        report.solid_area += r.solid as f64 * px;
        report.gentle_overhang_area += r.gentle as f64 * px;
        report.bridge_area += r.bridged as f64 * px;
        report.unsupported_interior_area += r.unsupported_interior as f64 * px;
        report.unsupported_exterior_area += r.unsupported_exterior as f64 * px;
        report.interior_islands += r.islands;
        report.interior_island_area += r.island_pixels as f64 * px;
        report.interior_specks += r.specks;
        report.longest_bridge = report.longest_bridge.max(r.longest_bridge as f64 * res);
        report.longest_rejected_span = report.longest_rejected_span.max(r.longest_rejected as f64 * res);
        per_layer.push((lo[2] + (k as f64 + 0.5) * h, r.unsupported_interior as f64 * px));
    }
    if report.solid_area > 0.0 {
        report.unsupported_interior_pct = 100.0 * report.unsupported_interior_area / report.solid_area;
    }
    per_layer.sort_by(|a, b| b.1.total_cmp(&a.1));
    report.worst_layers = per_layer.into_iter().take(5).filter(|l| l.1 > 0.0).collect();
    examples.sort_by(|a, b| b[3].total_cmp(&a[3]));
    examples.truncate(10);
    report.island_examples = examples;
    report.down_facing_area_by_angle = down_facing_histogram(mesh, reference, lo[2]);
    report
}

fn disc_offsets(radius: f32) -> Vec<(i32, i32)> {
    let r = radius.ceil() as i32;
    let mut out: Vec<(i32, i32, i32)> = Vec::new();
    for dj in -r..=r {
        for di in -r..=r {
            let d2 = di * di + dj * dj;
            if d2 > 0 && (d2 as f32).sqrt() < radius {
                out.push((di, dj, d2));
            }
        }
    }
    out.sort_by_key(|o| o.2);
    out.into_iter().map(|o| (o.0, o.1)).collect()
}

#[allow(clippy::too_many_arguments)]
fn analyze_layer(
    cur: &[u8],
    under: &[u8],
    under_ref: Option<&[u8]>,
    first_layer: bool,
    nx: usize,
    ny: usize,
    limit: f32,
    offsets: &[(i32, i32)],
    max_bridge_px: f32,
    min_band_px: f32,
    min_island_px: usize,
    keep_state: bool,
    collect_pixels: bool,
) -> LayerResult {
    let mut out = LayerResult::default();
    let mut state = vec![VOID; nx * ny];
    let mut any_unsupported = false;
    for j in 0..ny {
        for i in 0..nx {
            let p = j * nx + i;
            if cur[p] == 0 {
                continue;
            }
            out.solid += 1;
            if first_layer || under[p] != 0 {
                state[p] = SUPPORTED;
                continue;
            }
            let near = offsets.iter().any(|&(di, dj)| {
                let (ii, jj) = (i as i32 + di, j as i32 + dj);
                ii >= 0 && jj >= 0 && ii < nx as i32 && jj < ny as i32 && under[jj as usize * nx + ii as usize] != 0
            });
            if near && limit > 0.5 {
                state[p] = GENTLE;
                out.gentle += 1;
            } else {
                state[p] = UNSUPPORTED;
                any_unsupported = true;
            }
        }
    }
    if !any_unsupported {
        if keep_state {
            out.exterior = vec![false; nx * ny];
            out.state = state;
        }
        return out;
    }
    let mut exterior = if keep_state { vec![false; nx * ny] } else { Vec::new() };

    let bridges = bridge::mark_bridges(
        &mut state,
        nx,
        ny,
        max_bridge_px,
        min_band_px,
        Mode::PerPixel,
        &mut BridgeScratch::default(),
    );
    out.bridged = bridges.bridged;
    out.longest_bridge = bridges.longest;
    out.longest_rejected = bridges.longest_rejected;

    // Islands: connected regions (8-neighbour) of this layer that contain no
    // supported, gentle or bridged pixel at all.
    let mut visited = vec![false; nx * ny];
    let mut stack: Vec<usize> = Vec::new();
    let mut region: Vec<usize> = Vec::new();
    for start in 0..nx * ny {
        if state[start] != UNSUPPORTED || visited[start] {
            continue;
        }
        // Flood through unsupported pixels only; note whether the region
        // touches anything anchored.
        region.clear();
        stack.push(start);
        visited[start] = true;
        let mut anchored = false;
        while let Some(p) = stack.pop() {
            region.push(p);
            let (i, j) = ((p % nx) as i32, (p / nx) as i32);
            for dj in -1..=1 {
                for di in -1..=1 {
                    let (ii, jj) = (i + di, j + dj);
                    if (di == 0 && dj == 0) || ii < 0 || jj < 0 || ii >= nx as i32 || jj >= ny as i32 {
                        continue;
                    }
                    let q = jj as usize * nx + ii as usize;
                    match state[q] {
                        UNSUPPORTED => {
                            if !visited[q] {
                                visited[q] = true;
                                stack.push(q);
                            }
                        }
                        VOID => {}
                        _ => anchored = true,
                    }
                }
            }
        }
        // On the outside of the model, a piece in mid-air is the model's own
        // overhang, for the slicer to support, unless the model itself had
        // something to rest on within reach: then the infill took it away
        // (the skin along an opened face, say), and it counts as inside.
        let made_here = !anchored
            && under_ref.is_some_and(|r| {
                region.iter().any(|&p| {
                    let (i, j) = ((p % nx) as i32, (p / nx) as i32);
                    r[p] != 0
                        || offsets.iter().any(|&(di, dj)| {
                            let (ii, jj) = (i + di, j + dj);
                            ii >= 0
                                && jj >= 0
                                && ii < nx as i32
                                && jj < ny as i32
                                && r[jj as usize * nx + ii as usize] != 0
                        })
                })
            });
        let inside = |p: usize| made_here || under_ref.is_none_or(|r| r[p] != 0);
        let interior = region.iter().filter(|&&p| inside(p)).count();
        out.unsupported_interior += interior;
        out.unsupported_exterior += region.len() - interior;
        if collect_pixels {
            out.interior_pixels.extend(region.iter().filter(|&&p| inside(p)).map(|&p| p as u32));
        }
        if keep_state {
            for &p in &region {
                exterior[p] = !inside(p);
            }
        }
        if !anchored && region.len() < min_island_px {
            out.dust.extend(region.iter().map(|&p| p as u32));
        }
        if !anchored && interior * 2 > region.len() {
            if region.len() >= min_island_px {
                out.islands += 1;
                out.island_pixels += region.len();
                out.island_seeds.push((region[0], region.len()));
            } else {
                out.specks += 1;
            }
        }
    }
    if keep_state {
        out.state = state;
        out.exterior = exterior;
    }
    out
}

/// A picture of one layer as the analysis sees it.
pub struct LayerMap {
    pub width: usize,
    pub height: usize,
    /// RGB8, row 0 at the top (+Y up).
    pub rgb: Vec<u8>,
    /// Height of the layer that was drawn.
    pub z: f64,
}

/// Classifies the layer at height `z` and draws it: grey = resting on the
/// layer below, green = gentle overhang, blue = bridge, red = unsupported
/// (introduced by the infill), orange = unsupported in the original model too.
pub fn layer_map(mesh: &TriMesh, reference: Option<&TriMesh>, p: &AnalyzeParams, z: f64) -> LayerMap {
    let (lo, hi) = mesh.bounds();
    let h = p.layer_height;
    let res = p.resolution;
    let margin_px = 3.0;
    let nx = ((hi[0] - lo[0]) / res).ceil() as usize + 2 * margin_px as usize + 1;
    let ny = ((hi[1] - lo[1]) / res).ceil() as usize + 2 * margin_px as usize + 1;
    let layer = nx * ny;
    // Snap to the centre of the printed layer containing z.
    let k = (((z - lo[2]) / h).floor().max(1.0)) as usize;
    // Layers k - 2, k - 1 and k: the one below k is checked too, so that dust
    // on it is not taken as support, as in `analyze` (one layer deep only).
    let grid = Grid {
        origin: [lo[0] - margin_px * res, lo[1] - margin_px * res, lo[2] + (k as f64 - 1.5) * h],
        spacing: [res, res, h],
        dims: [nx, ny, 3],
    };
    let solid = voxelize(mesh, &grid);
    let solid_ref = reference.map(|r| voxelize(r, &grid));
    let limit = (h * p.max_overhang_deg.to_radians().tan() / res) as f32 + 0.5;
    let offsets = disc_offsets(limit);
    let classify = |layer_index: usize, under: &[u8], keep_state: bool| {
        analyze_layer(
            &solid[layer_index * layer..(layer_index + 1) * layer],
            under,
            solid_ref.as_ref().map(|r| &r[(layer_index - 1) * layer..layer_index * layer]),
            k == 1 && layer_index == 1,
            nx,
            ny,
            limit,
            &offsets,
            (p.max_bridge / res) as f32,
            (p.min_bridge_width / res) as f32,
            (p.min_island_area / (res * res)).ceil() as usize,
            keep_state,
            false,
        )
    };
    let mut under = solid[layer..2 * layer].to_vec();
    for p in classify(1, &solid[..layer], false).dust {
        under[p as usize] = 0;
    }
    let result = classify(2, &under, true);
    let solid = &solid[layer..];
    let mut rgb = vec![0u8; layer * 3];
    for j in 0..ny {
        for i in 0..nx {
            let idx = j * nx + i;
            let colour: [u8; 3] = match result.state[idx] {
                SUPPORTED => [150, 150, 150],
                GENTLE => [70, 170, 90],
                BRIDGED => [60, 120, 230],
                UNSUPPORTED if result.exterior[idx] => [240, 160, 40],
                UNSUPPORTED => [235, 40, 40],
                // Empty here; show faintly where the layer below has material.
                _ if solid[idx] != 0 => [45, 45, 50],
                _ => [20, 17, 15],
            };
            let dst = ((ny - 1 - j) * nx + i) * 3;
            rgb[dst..dst + 3].copy_from_slice(&colour);
        }
    }
    LayerMap { width: nx, height: ny, rgb, z: lo[2] + (k as f64 + 0.5) * h }
}

/// Down-facing area by overhang angle. With a reference model, faces of the
/// original outer surface are skipped.
fn down_facing_histogram(mesh: &TriMesh, reference: Option<&TriMesh>, z_bed: f64) -> [f64; 5] {
    let probe = reference.map(|r| {
        let (lo, hi) = r.bounds();
        let size = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
        let voxel = (size / 400.0).max(0.2);
        let grid = Grid::enclosing(lo, hi, voxel, 3.0 * voxel);
        let occupancy = voxelize(r, &grid);
        (grid, occupancy, voxel)
    });
    mesh.triangles
        .par_iter()
        .fold(
            || [0f64; 5],
            |mut acc, t| {
                let n = area_normal(&mesh.vertices, t);
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                if len == 0.0 || n[2] >= 0.0 {
                    return acc;
                }
                let v = [mesh.vertices[t[0] as usize], mesh.vertices[t[1] as usize], mesh.vertices[t[2] as usize]];
                if v.iter().all(|p| (p[2] as f64) < z_bed + 0.05) {
                    return acc; // resting on the build plate
                }
                if let Some((grid, occupancy, voxel)) = &probe {
                    // Step off the surface along its normal: for the model's
                    // own outer surface that lands outside the reference.
                    let step = 1.5 * voxel / len;
                    let c = [0, 1, 2].map(|a| (v[0][a] as f64 + v[1][a] as f64 + v[2][a] as f64) / 3.0 + n[a] * step);
                    let inside = grid.nearest(c).is_some_and(|idx| occupancy[grid.index(idx[0], idx[1], idx[2])] != 0);
                    if !inside {
                        return acc;
                    }
                }
                let angle = (-n[2] / len).clamp(0.0, 1.0).asin().to_degrees();
                let bin = match angle {
                    a if a < 30.0 => 0,
                    a if a < 45.0 => 1,
                    a if a < 60.0 => 2,
                    a if a < 75.0 => 3,
                    _ => 4,
                };
                acc[bin] += 0.5 * len;
                acc
            },
        )
        .reduce(|| [0f64; 5], |a, b| std::array::from_fn(|i| a[i] + b[i]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    fn params() -> AnalyzeParams {
        AnalyzeParams { resolution: 0.25, ..AnalyzeParams::default() }
    }

    #[test]
    fn solid_box_is_fully_printable() {
        let m = box_mesh([0.0; 3], [10.0, 8.0, 6.0]);
        let r = analyze(&m, None, &params());
        assert_eq!(r.layers, 30);
        assert_eq!(r.unsupported_interior_area, 0.0);
        assert_eq!(r.interior_islands, 0);
        assert!((r.solid_area - 80.0 * 30.0).abs() / 2400.0 < 0.02);
    }

    #[test]
    fn mushroom_cap_is_unsupported() {
        // A wide slab on a narrow stem: the cap's first layer overhangs.
        let mut m = box_mesh([8.0, 8.0, 0.0], [12.0, 12.0, 5.0]);
        m.append(&box_mesh([0.0, 0.0, 5.0], [20.0, 20.0, 7.0]));
        let r = analyze(&m, None, &params());
        // Cap area minus the stem and the ring within the overhang limit.
        assert!(r.unsupported_interior_area > 300.0, "{r:?}");
        assert_eq!(r.interior_islands, 0);
        // With the same shape as reference, it is all "exterior".
        let r2 = analyze(&m, Some(&m), &params());
        assert_eq!(r2.unsupported_interior_area, 0.0);
        assert!(r2.unsupported_exterior_area > 300.0);
    }

    #[test]
    fn floating_block_is_an_island_and_a_table_is_a_bridge() {
        let mut m = box_mesh([0.0, 0.0, 0.0], [10.0, 10.0, 1.0]);
        m.append(&box_mesh([4.0, 4.0, 3.0], [6.0, 6.0, 4.0]));
        let r = analyze(&m, None, &params());
        assert_eq!(r.interior_islands, 1);
        assert!((r.interior_island_area - 4.0).abs() < 1.0);

        // Two legs 3 mm apart with a top across them.
        let mut t = box_mesh([0.0, 0.0, 0.0], [2.0, 6.0, 4.0]);
        t.append(&box_mesh([5.0, 0.0, 0.0], [7.0, 6.0, 4.0]));
        t.append(&box_mesh([0.0, 0.0, 4.0], [7.0, 6.0, 5.0]));
        let r = analyze(&t, None, &params());
        assert_eq!(r.unsupported_interior_area, 0.0, "{r:?}");
        assert!(r.bridge_area > 10.0);
        assert!(r.longest_bridge > 2.0 && r.longest_bridge <= 4.0);
        // The same table with a 2 mm bridge limit fails.
        let strict = AnalyzeParams { max_bridge: 2.0, ..params() };
        let r = analyze(&t, None, &strict);
        assert!(r.unsupported_interior_area > 5.0);
        assert!(r.longest_rejected_span > 2.0);
    }

    #[test]
    fn a_block_resting_only_on_a_speck_is_an_island() {
        // A 0.3 mm speck in mid-air, too small to print, and a 1 mm block on
        // top of it: the block starts in mid-air all the same.
        let mut m = box_mesh([0.0, 0.0, 0.0], [10.0, 10.0, 1.0]);
        m.append(&box_mesh([4.85, 4.85, 2.0], [5.15, 5.15, 2.2]));
        m.append(&box_mesh([4.5, 4.5, 2.2], [5.5, 5.5, 3.0]));
        for p in [AnalyzeParams::default(), AnalyzeParams { layer_height: 0.1, ..AnalyzeParams::default() }] {
            let r = analyze(&m, None, &p);
            assert_eq!(r.interior_islands, 1, "{r:?}");
            assert!((r.interior_island_area - 1.0).abs() < 0.3, "{r:?}");
            // The map at the block's first layer agrees.
            let map = layer_map(&m, None, &p, 2.3);
            let red = map.rgb.chunks(3).filter(|c| c == &[235, 40, 40]).count();
            assert!(red as f64 * p.resolution * p.resolution > 0.7, "{red}");
        }
    }

    #[test]
    fn slope_within_limit_passes_and_beyond_fails() {
        // A leaning prism: sheared box, offset `shear` mm over 10 mm height.
        let leaning = |shear: f32| {
            let mut m = box_mesh([0.0, 0.0, 0.0], [6.0, 6.0, 10.0]);
            for v in &mut m.vertices {
                v[0] += shear * v[2] / 10.0;
            }
            m
        };
        let ok = analyze(&leaning(5.0), None, &params()); // 26.6 degrees
        assert_eq!(ok.unsupported_interior_area, 0.0, "{ok:?}");
        let bad = analyze(&leaning(25.0), None, &params()); // 68 degrees
        assert!(bad.unsupported_interior_area > 5.0, "{bad:?}");
        assert!(bad.down_facing_area_by_angle[3] > 50.0);
    }
}
