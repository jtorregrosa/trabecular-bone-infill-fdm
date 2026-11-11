//! The end-to-end generation pipeline.

use crate::field::{self, Calibration};
use crate::grid::Grid;
use crate::isosurface;
use crate::mesh::{EdgeReport, TriMesh};
use crate::params::Params;
use crate::sdf;
use crate::simplify;
use crate::support::{self, SupportStats};
use crate::util::now_seconds;
use crate::vault;
use crate::volume::Volume;
use anyhow::{Result, bail};
use rayon::prelude::*;
use serde::Serialize;

/// PLA density, g/cm³, for the weight estimate.
const PLA_DENSITY: f64 = 1.24;

#[derive(Clone, Debug, Serialize)]
pub struct GenerateReport {
    pub params: Params,
    pub model_size: [f64; 3],
    pub model_volume: f64,
    pub grid: [usize; 3],
    pub voxel: f64,
    pub voxels: usize,
    pub core_fraction: f64,
    pub calibration: Calibration,
    /// Number of field evaluations (more than one when density compensation
    /// had to correct the target).
    pub passes: usize,
    /// Solid fraction of the lattice region in the final part.
    pub lattice_density: f64,
    /// Density inside and outside the reinforced spheres, when there are any.
    pub load_zones: Option<field::LoadZones>,
    /// Volume carved by the overhang filter (mm³).
    pub removed_unsupported: f64,
    /// Volume kept as bridges (mm³).
    pub bridged: f64,
    /// Volume of thin features removed (mm³).
    pub removed_thin: f64,
    /// Volume of loose pieces removed (mm³).
    pub removed_loose: f64,
    /// Volume of the vaults added under inner ceilings (mm³).
    pub vaults: f64,
    pub triangles_before_simplify: usize,
    pub triangles: usize,
    pub topology: EdgeReport,
    pub volume: f64,
    pub volume_fraction: f64,
    pub weight_pla_g: f64,
    pub dropped_components: usize,
    /// `(stage, seconds)`.
    pub timings: Vec<(String, f64)>,
    pub total_seconds: f64,
}

pub struct Generated {
    pub mesh: TriMesh,
    pub report: GenerateReport,
}

/// Checks and repairs an input mesh: caps simple holes and makes normals
/// point outwards. Returns the mesh and human-readable notes about anything
/// that was changed or looks wrong.
pub fn prepare_model(mut mesh: TriMesh) -> Result<(TriMesh, Vec<String>)> {
    let mut notes = Vec::new();
    let mut report = mesh.edge_report();
    if report.boundary > 0 {
        let holes = mesh.fill_holes();
        if holes > 0 {
            notes.push(format!("open mesh: capped {holes} hole(s)"));
            report = mesh.edge_report();
        }
        if report.boundary > 0 {
            bail!(
                "the mesh is not closed ({} open edges that could not be capped); repair it first, e.g. in your slicer",
                report.boundary
            );
        }
    }
    if report.non_manifold > 0 || report.misoriented > 0 {
        notes.push(format!(
            "mesh is not a clean manifold ({} non-manifold, {} misoriented edges); continuing, but check the result",
            report.non_manifold, report.misoriented
        ));
    }
    let volume = mesh.signed_volume();
    if volume == 0.0 {
        bail!("the mesh encloses no volume");
    }
    if volume < 0.0 {
        mesh.flip();
        notes.push("mesh was inside out: flipped".to_string());
    }
    // A shell with negative volume is a cavity. A model that already has
    // them is usually the output of an earlier run, or a hollowed part.
    let (_, components) = mesh.components();
    let cavities = components.iter().filter(|c| c.volume < 0.0).count();
    if cavities > 0 {
        notes.push(format!(
            "the model already has {cavities} internal cavit{}: is it hollow, or an earlier result? Infill only goes where it is solid",
            if cavities == 1 { "y" } else { "ies" }
        ));
    }
    Ok((mesh, notes))
}

struct Clock {
    start: f64,
    last: f64,
    timings: Vec<(String, f64)>,
}

impl Clock {
    fn new() -> Self {
        let now = now_seconds();
        Self { start: now, last: now, timings: Vec::new() }
    }
    fn lap(&mut self, stage: &str, log: &dyn Fn(&str)) {
        let now = now_seconds();
        let dt = now - self.last;
        self.last = now;
        // Repeated stages (density compensation) accumulate.
        match self.timings.iter_mut().find(|t| t.0 == stage) {
            Some(t) => t.1 += dt,
            None => self.timings.push((stage.to_string(), dt)),
        }
        log(&format!("  {stage:<22} {dt:6.2} s"));
    }
}

/// Runs the whole pipeline on a prepared (closed, outward-oriented) model.
/// `log` receives progress lines.
pub fn generate(model: &TriMesh, p: &Params, log: &dyn Fn(&str)) -> Result<Generated> {
    p.validate()?;
    let mut clock = Clock::new();
    let (lo, hi) = model.bounds();
    let size = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let model_volume = model.signed_volume();

    // Grid, enlarging the voxel if the part would blow the budget.
    let mut voxel = p.voxel;
    let estimate = |v: f64| size.iter().map(|s| (s + 2.0 * (2.0 * v + 1.0)) / v + 2.0).product::<f64>();
    if estimate(voxel) > p.max_voxels {
        // Solve by a few fixed-point steps (the margin depends on the voxel).
        for _ in 0..8 {
            voxel *= (estimate(voxel) / p.max_voxels).cbrt();
        }
        log(&format!("large part: voxel enlarged to {voxel:.3} mm to stay within the voxel budget"));
    }
    let grid = Grid::enclosing(lo, hi, voxel, 2.0 * voxel + 1.0);
    if p.cell < 6.0 * voxel {
        bail!("--cell ({} mm) is too small for the effective voxel ({voxel:.3} mm)", p.cell);
    }
    log(&format!(
        "model {:.1} x {:.1} x {:.1} mm, {:.1} cm3; grid {} x {} x {} = {:.1} M voxels of {:.3} mm",
        size[0],
        size[1],
        size[2],
        model_volume / 1000.0,
        grid.nx(),
        grid.ny(),
        grid.nz(),
        grid.len() as f64 / 1e6,
        voxel
    ));

    let vol = Volume::build(model, grid, p);
    clock.lap("voxelize + distances", log);
    if vol.core_voxels == 0 {
        bail!("the part is too thin for a {} mm skin: there is no interior left to fill", p.shell);
    }
    let voxel_volume = voxel.powi(3);

    // Open faces: the lattice is cut by the model surface itself. The cut is
    // applied to the field before the printability passes, not only before
    // meshing: they must judge the material that will actually be printed.
    // Uncut, a strut ending on a down-facing open surface looks deeper than
    // it is and props up the layer above it, so the cut part ends up starting
    // in mid-air.
    let open_cut = (!p.open.is_empty()).then(|| {
        let exact = sdf::narrow_band(model, &vol.grid, &vol.flags, 2.5 * voxel as f32);
        clock.lap("open-face distance", log);
        exact
    });

    // Field + printability. The printability passes remove material, so the
    // field is regenerated with a corrected target until the lattice density
    // lands on the requested one (secant iteration, usually one extra pass).
    const MAX_PASSES: usize = 4;
    let mut history: Vec<(f64, f64)> = Vec::new(); // (target, measured)
    let mut target = p.density;
    let (solid, recipe, calibration, stats, thin, loose, vault_voxels, density) = loop {
        let (recipe, calibration) = field::calibrate(&vol, p, target);
        clock.lap("calibrate", log);
        let mut solid = field::evaluate(&vol, &recipe);
        if let Some(exact) = &open_cut {
            solid.par_iter_mut().zip(exact.par_iter()).for_each(|(s, &d)| *s = s.min(d));
        }
        clock.lap("bone field", log);

        // Carve first: the vaults must spring from lattice that will still
        // be there, not from pieces that only hang from the roof.
        let mut stats = support::make_printable(&vol, &mut solid, p);
        clock.lap("printability", log);
        let (vault_mask, vault_voxels) = vault::build_vaults(&vol, &mut solid, p);
        clock.lap("vaults", log);
        if vault_voxels > 0 {
            let again = support::make_printable(&vol, &mut solid, p);
            stats = SupportStats {
                removed_voxels: stats.removed_voxels + again.removed_voxels,
                bridged_voxels: again.bridged_voxels,
            };
        }
        let thin = if p.min_feature > 0.0 { support::remove_thin(&vol, &mut solid, p.min_feature) } else { 0 };
        if thin > 0 {
            // Removing slivers can take away what something else stood on.
            let again = support::make_printable(&vol, &mut solid, p);
            stats = SupportStats {
                removed_voxels: stats.removed_voxels + again.removed_voxels,
                bridged_voxels: again.bridged_voxels,
            };
        }
        let loose = support::remove_loose(&vol, &mut solid);
        clock.lap("printability", log);

        // Vaults are deliberate extra material: leave them out of the density.
        let density = field::core_density(&vol, &solid, Some(&vault_mask));
        history.push((target, density));
        let miss = (density - p.density).abs() / p.density;
        if !p.compensate_density || history.len() >= MAX_PASSES || miss <= 0.03 {
            break (solid, recipe, calibration, stats, thin, loose, vault_voxels, density);
        }
        let next = match history.as_slice() {
            [.., (t0, m0), (t1, m1)] if (m1 - m0).abs() > 1e-6 => t1 + (p.density - m1) * (t1 - t0) / (m1 - m0),
            _ => target * p.density / density.max(1e-6),
        }
        .clamp(0.02, 0.9);
        if (next - target).abs() < 1e-4 {
            break (solid, recipe, calibration, stats, thin, loose, vault_voxels, density);
        }
        log(&format!("  lattice density {:.1} % vs target {:.1} %: regenerating", 100.0 * density, 100.0 * p.density));
        target = next;
    };
    let passes = history.len();
    let load_zones = field::load_zones(&vol, &solid, &recipe, &p.loads);

    // Surface.
    let mut surface = match &open_cut {
        // Closed part: only the internal cavities are meshed; the original
        // surface is reused untouched.
        None => isosurface::extract(&solid, &vol.grid),
        // Open faces: the whole part is re-meshed against the exact cut. The
        // field already holds it; applying it again keeps whatever the
        // printability passes added (vaults) inside the model too.
        Some(exact) => {
            let mut cut = solid;
            cut.par_iter_mut().zip(exact.par_iter()).for_each(|(s, &d)| *s = s.min(d));
            isosurface::extract(&cut, &vol.grid)
        }
    };
    clock.lap("marching cubes", log);
    let triangles_before_simplify = surface.triangles.len();

    let tolerance = p.simplify_tolerance(voxel);
    if tolerance > 0.0 {
        surface = simplify::simplify(&surface, tolerance as f32);
        clock.lap("simplify", log);
    }

    let dropped_components = drop_crumbs(&mut surface, p.open.is_empty());
    let mesh = if p.open.is_empty() {
        let mut out = model.clone();
        out.append(&surface);
        out
    } else {
        surface
    };
    let topology = mesh.edge_report();
    let volume = mesh.signed_volume();
    clock.lap("assemble + verify", log);

    let report = GenerateReport {
        params: p.clone(),
        model_size: size,
        model_volume,
        grid: vol.grid.dims,
        voxel,
        voxels: vol.grid.len(),
        core_fraction: vol.core_voxels as f64 / vol.inside_voxels.max(1) as f64,
        calibration,
        passes,
        lattice_density: density,
        load_zones,
        removed_unsupported: stats.removed_voxels as f64 * voxel_volume,
        bridged: stats.bridged_voxels as f64 * voxel_volume,
        removed_thin: thin as f64 * voxel_volume,
        removed_loose: loose as f64 * voxel_volume,
        vaults: vault_voxels as f64 * voxel_volume,
        triangles_before_simplify,
        triangles: mesh.triangles.len(),
        topology,
        volume,
        volume_fraction: volume / model_volume,
        weight_pla_g: volume / 1000.0 * PLA_DENSITY,
        dropped_components,
        timings: clock.timings.clone(),
        total_seconds: now_seconds() - clock.start,
    };
    Ok(Generated { mesh, report })
}

/// Removes tiny disconnected shells from the extracted surface.
///
/// With `cavities_only` the surface consists of cavity walls (negative
/// volume). A shell with positive volume is then a solid crumb floating in a
/// pore: it is dropped whatever its size. Otherwise the largest positive
/// shell is the part and every other positive shell is a loose piece. Cavities
/// smaller than 0.5 mm³ are dropped in both cases (they become solid).
fn drop_crumbs(mesh: &mut TriMesh, cavities_only: bool) -> usize {
    let (labels, comps) = mesh.components();
    if comps.len() <= 1 {
        return 0;
    }
    let largest_positive = comps
        .iter()
        .enumerate()
        .filter(|(_, c)| c.volume > 0.0)
        .max_by(|a, b| a.1.volume.total_cmp(&b.1.volume))
        .map(|(i, _)| i);
    let keep: Vec<bool> = comps
        .iter()
        .enumerate()
        .map(|(i, c)| if c.volume > 0.0 { !cavities_only && Some(i) == largest_positive } else { c.volume < -0.5 })
        .collect();
    let dropped = keep.iter().filter(|k| !**k).count();
    if dropped > 0 {
        mesh.retain_triangles(|t| keep[labels[t] as usize]);
    }
    dropped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyze::{AnalyzeParams, analyze};
    use crate::mesh::box_mesh;
    use crate::params::Face;

    fn quick() -> Params {
        Params { voxel: 0.5, cell: 10.0, shell: 1.5, waves: 48, ..Params::default() }
    }

    #[test]
    fn closed_cube_keeps_its_exact_surface_and_is_watertight() {
        let model = box_mesh([0.0; 3], [30.0; 3]);
        let out = generate(&model, &quick(), &|_| {}).unwrap();
        assert!(out.report.topology.is_watertight(), "{:?}", out.report.topology);
        // The first vertices and triangles are the untouched input.
        assert_eq!(&out.mesh.vertices[..8], &model.vertices[..]);
        assert_eq!(&out.mesh.triangles[..12], &model.triangles[..]);
        // Hollowed, but the skin alone is already a good share of a small cube.
        assert!(
            out.report.volume_fraction > 0.25 && out.report.volume_fraction < 0.75,
            "{}",
            out.report.volume_fraction
        );
        let target = out.report.params.density;
        assert!((out.report.lattice_density - target).abs() < 0.03, "{} vs {target}", out.report.lattice_density);
    }

    #[test]
    fn open_cube_is_one_watertight_piece() {
        let model = box_mesh([0.0; 3], [30.0; 3]);
        let p = Params { open: vec![Face { axis: 2, positive: true }], ..quick() };
        let out = generate(&model, &p, &|_| {}).unwrap();
        assert!(out.report.topology.is_watertight(), "{:?}", out.report.topology);
        let (_, comps) = out.mesh.components();
        assert_eq!(comps.iter().filter(|c| c.volume > 0.0).count(), 1);
        let (lo, hi) = out.mesh.bounds();
        for a in 0..3 {
            assert!(lo[a] > -0.05 && hi[a] < 30.05);
        }
    }

    #[test]
    fn generated_lattice_passes_the_independent_printability_check() {
        let model = box_mesh([0.0; 3], [30.0; 3]);
        let check =
            AnalyzeParams { resolution: 0.2, max_overhang_deg: 50.0, max_bridge: 5.0, ..AnalyzeParams::default() };
        // With pores and holes stretched along Z, the rules leave nothing in mid-air.
        let stretched =
            Params { open: vec![Face { axis: 2, positive: true }], z_elongation: 2.0, hole_elongation: 2.0, ..quick() };
        let r = analyze(&generate(&model, &stretched, &|_| {}).unwrap().mesh, Some(&model), &check);
        assert_eq!(r.interior_islands, 0, "{r:?}");
        assert!(r.unsupported_interior_pct < 0.5, "{r:?}");
        // The rounder defaults trade a little of that for looks: specks at most.
        let round = Params { open: vec![Face { axis: 2, positive: true }], ..quick() };
        let r = analyze(&generate(&model, &round, &|_| {}).unwrap().mesh, Some(&model), &check);
        assert!(r.interior_island_area < 2.0, "{r:?}");
        assert!(r.unsupported_interior_pct < 0.5, "{r:?}");
    }

    #[test]
    fn too_thin_part_is_rejected() {
        let model = box_mesh([0.0; 3], [30.0, 30.0, 2.0]);
        assert!(generate(&model, &quick(), &|_| {}).is_err());
    }

    #[test]
    fn inside_out_and_open_inputs_are_repaired() {
        let mut m = box_mesh([0.0; 3], [5.0; 3]);
        m.flip();
        let (fixed, notes) = prepare_model(m).unwrap();
        assert!(fixed.signed_volume() > 0.0);
        assert_eq!(notes.len(), 1);
        let mut open = box_mesh([0.0; 3], [5.0; 3]);
        open.triangles.truncate(10);
        let (fixed, notes) = prepare_model(open).unwrap();
        assert!(fixed.edge_report().is_watertight());
        assert!(notes[0].contains("capped"));
        // A part that is already hollow is accepted, with a warning.
        let mut hollow = box_mesh([0.0; 3], [10.0; 3]);
        let mut cavity = box_mesh([2.0; 3], [8.0; 3]);
        cavity.flip();
        hollow.append(&cavity);
        let (_, notes) = prepare_model(hollow).unwrap();
        assert!(notes.iter().any(|n| n.contains("1 internal cavity")), "{notes:?}");
        assert!(prepare_model(box_mesh([0.0; 3], [5.0; 3])).unwrap().1.is_empty());
    }
}
