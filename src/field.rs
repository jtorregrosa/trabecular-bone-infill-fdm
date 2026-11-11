//! The bone field: a scalar field, in millimetres, that is positive where
//! there is material.
//!
//! Trabecular bone is modelled as perforated sheets:
//!
//! * a first random field `g1` gives undulating sheets, the set where the
//!   distance to its zero level, `|g1| / |grad g1|`, is below half the wall
//!   thickness (dividing by the gradient makes walls uniformly thick);
//! * a second, finer field `g2` punches holes through them (`g2 < t`), which
//!   turns plates into perforated plates and rods where material is scarce;
//! * wall thickness and perforation follow a density map that is higher near
//!   the skin and inside user-defined load regions;
//! * the skin is merged in with a smooth union so junctions get fillets.

use crate::filters::smooth3;
use crate::grid::flag;
use crate::mathx::{ndtri, quantile_in_place, smooth_max};
use crate::noise::{RowBuffers, WaveNoise, WaveTables};
use crate::params::{Load, Params};
use crate::util::Rng;
use crate::volume::Volume;
use rayon::prelude::*;
use serde::Serialize;

/// Range and resolution of the density-level lookup tables.
const LUT_MIN: f32 = -6.0;
const LUT_MAX: f32 = 9.0;
const LUT_SIZE: usize = 3000;
/// Calibration sample size.
const SAMPLES: usize = 200_000;

#[derive(Clone, Debug, Serialize)]
pub struct Calibration {
    /// Global offset of the density level found by bisection.
    pub shift: f32,
    /// Lattice solid fraction predicted from the calibration sample.
    pub predicted_density: f64,
    /// Mean sheet thickness over the sample (mm).
    pub mean_wall: f64,
    /// Thinnest sheet thickness over the sample (mm).
    pub min_wall: f64,
    /// True when `min_wall` forced thicker sheets than the density asked for.
    pub wall_clamped: bool,
}

/// Everything needed to evaluate the lattice at a voxel.
pub struct Recipe {
    shell: f32,
    blend: f32,
    skin_boost: f32,
    inv_skin_scale: f32,
    loads: Vec<([f32; 3], f32)>,
    load_boost: f32,
    inv_load_scale: f32,
    lo: f32,
    inv_span: f32,
    pub shift: f32,
    /// Half wall thickness (mm) by density level.
    lut_half_wall: Vec<f32>,
    /// Perforation threshold on `g2` by density level.
    lut_hole: Vec<f32>,
    grad_floor_sq: f32,
    hole_mm: f32,
    pub sheets: WaveNoise,
    pub holes: WaveNoise,
}

impl Recipe {
    /// Relative material demand at a point.
    #[inline]
    fn rho(&self, dist: f32, p: [f32; 3]) -> f32 {
        let mut r = 1.0 + self.skin_boost * (-(dist - self.shell).max(0.0) * self.inv_skin_scale).exp();
        for (c, radius) in &self.loads {
            let d = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) + (p[2] - c[2]).powi(2)).sqrt();
            r += self.load_boost * (-(d - radius).max(0.0) * self.inv_load_scale).exp();
        }
        r
    }

    #[inline]
    fn level(&self, rho: f32) -> f32 {
        (rho - self.lo) * self.inv_span + self.shift
    }

    #[inline]
    fn lut(table: &[f32], level: f32) -> f32 {
        let pos = ((level - LUT_MIN) * ((LUT_SIZE - 1) as f32 / (LUT_MAX - LUT_MIN)))
            .clamp(0.0, (LUT_SIZE - 1) as f32 - 1e-3);
        let i = pos as usize;
        let t = pos - i as f32;
        table[i] + (table[i + 1] - table[i]) * t
    }

    /// Lattice field (mm, positive = material), before merging with the skin.
    #[inline]
    fn lattice(&self, level: f32, g1: f32, grad_sq: f32, g2: f32) -> f32 {
        let sheet_distance = g1.abs() / grad_sq.max(self.grad_floor_sq).sqrt();
        let sheet = Self::lut(&self.lut_half_wall, level) - sheet_distance;
        let holes = (g2 - Self::lut(&self.lut_hole, level)) * self.hole_mm;
        sheet.min(holes)
    }
}

/// Sheet fraction and perforated fraction as a function of the density level.
/// Between levels 0 and 1 the sheet fraction reproduces the proportions that
/// looked right in the prototype, while the perforation goes from its
/// "sparse" to its "dense" setting. Outside that range only the wall
/// thickness keeps changing, so that any target density can be reached.
fn fractions(level: f64, perforation: [f64; 2]) -> (f64, f64) {
    let sheet = (0.23 + 0.11 * level).clamp(0.02, 0.92);
    let holes = perforation[0] + (perforation[1] - perforation[0]) * level.clamp(0.0, 1.0);
    (sheet, holes)
}

/// Builds the recipe and calibrates it so the lattice hits `target_density`.
pub fn calibrate(vol: &Volume, p: &Params, target_density: f64) -> (Recipe, Calibration) {
    let mut rng = Rng::new(p.seed);
    let stretch = p.stretch_dir.map(|d| (d, p.stretch));
    let hole_wavelength = p.cell * p.hole_ratio;
    let sheets = WaveNoise::new(&mut rng, p.cell, p.waves, p.bandwidth, p.z_elongation, stretch);
    let holes = WaveNoise::new(&mut rng, hole_wavelength, p.waves, p.bandwidth, p.hole_elongation, None);

    let grad_rms = sheets.grad_rms();
    let half_min_wall = 0.5 * p.min_wall;
    let mut lut_half_wall = Vec::with_capacity(LUT_SIZE);
    let mut lut_hole = Vec::with_capacity(LUT_SIZE);
    for i in 0..LUT_SIZE {
        let level = LUT_MIN as f64 + (LUT_MAX - LUT_MIN) as f64 * i as f64 / (LUT_SIZE - 1) as f64;
        let (sheet, hole) = fractions(level, p.perforation);
        // P(|g| < w) = sheet for a unit Gaussian  =>  w = ndtri((1 + sheet) / 2);
        // dividing by the RMS gradient converts it to millimetres.
        let half = ndtri(0.5 * (1.0 + sheet)) / grad_rms;
        lut_half_wall.push(half.max(half_min_wall) as f32);
        lut_hole.push(ndtri(hole.max(1e-6)) as f32);
    }

    let mut recipe = Recipe {
        shell: p.shell as f32,
        blend: p.blend as f32,
        skin_boost: p.skin_boost as f32,
        inv_skin_scale: 1.0 / (0.35 * p.cell) as f32,
        loads: p
            .loads
            .iter()
            .map(|l| ([l.center[0] as f32, l.center[1] as f32, l.center[2] as f32], l.radius as f32))
            .collect(),
        load_boost: p.load_boost as f32,
        inv_load_scale: 1.0 / (0.4 * p.cell) as f32,
        lo: 0.0,
        inv_span: 1.0,
        shift: 0.0,
        lut_half_wall,
        lut_hole,
        grad_floor_sq: (0.35 * grad_rms).powi(2) as f32,
        hole_mm: (hole_wavelength / std::f64::consts::TAU) as f32,
        sheets,
        holes,
    };

    // Sample of core voxels: every `stride`-th one, starting at a random offset.
    let stride = (vol.core_voxels / SAMPLES).max(1);
    let mut countdown = Rng::new(p.seed ^ 0x5EED).below(stride);
    let mut picks: Vec<usize> = Vec::with_capacity(vol.core_voxels / stride + 1);
    for idx in 0..vol.flags.len() {
        if vol.is_core(idx) {
            if countdown == 0 {
                picks.push(idx);
                countdown = stride;
            }
            countdown -= 1;
        }
    }
    if picks.is_empty() {
        let cal =
            Calibration { shift: 0.0, predicted_density: 0.0, mean_wall: 0.0, min_wall: 0.0, wall_clamped: false };
        return (recipe, cal);
    }

    struct Sample {
        rho: f32,
        g1: f32,
        grad_sq: f32,
        g2: f32,
    }
    let grid = &vol.grid;
    let [nx, ny, _] = grid.dims;
    let samples: Vec<Sample> = picks
        .par_iter()
        .map(|&idx| {
            let (i, j, k) = (idx % nx, (idx / nx) % ny, idx / (nx * ny));
            let pos = [grid.x(i), grid.y(j), grid.z(k)];
            let (g1, grad) = recipe.sheets.eval(pos);
            let (g2, _) = recipe.holes.eval(pos);
            Sample {
                rho: recipe.rho(vol.dist[idx], [pos[0] as f32, pos[1] as f32, pos[2] as f32]),
                g1,
                grad_sq: grad[0] * grad[0] + grad[1] * grad[1] + grad[2] * grad[2],
                g2,
            }
        })
        .collect();

    // Normalise the demand map to its 5..95 % range over the core.
    let mut rhos: Vec<f32> = samples.iter().map(|s| s.rho).collect();
    let lo = quantile_in_place(&mut rhos, 0.05);
    let hi = quantile_in_place(&mut rhos, 0.95);
    recipe.lo = lo;
    recipe.inv_span = 1.0 / (hi - lo).max(1e-6);

    let density_at = |recipe: &Recipe| -> f64 {
        let solid =
            samples.par_iter().filter(|s| recipe.lattice(recipe.level(s.rho), s.g1, s.grad_sq, s.g2) > 0.0).count();
        solid as f64 / samples.len() as f64
    };
    let (mut a, mut b) = (LUT_MIN + 1.0, LUT_MAX - 2.0);
    for _ in 0..32 {
        recipe.shift = 0.5 * (a + b);
        if density_at(&recipe) < target_density {
            a = recipe.shift;
        } else {
            b = recipe.shift;
        }
    }
    recipe.shift = 0.5 * (a + b);

    let walls: Vec<f64> =
        samples.iter().map(|s| 2.0 * Recipe::lut(&recipe.lut_half_wall, recipe.level(s.rho)) as f64).collect();
    let min_wall = walls.iter().copied().fold(f64::INFINITY, f64::min);
    let cal = Calibration {
        shift: recipe.shift,
        predicted_density: density_at(&recipe),
        mean_wall: walls.iter().sum::<f64>() / walls.len() as f64,
        min_wall,
        wall_clamped: min_wall <= p.min_wall * 1.0001 && p.min_wall > 0.0,
    };
    (recipe, cal)
}

/// Evaluates the solid field on the whole grid: lattice merged with the skin.
///
/// Outside the model the value is `shell` (positive), so a surface extracted
/// from this field alone contains only the internal cavities.
pub fn evaluate(vol: &Volume, recipe: &Recipe) -> Vec<f32> {
    let grid = &vol.grid;
    let [nx, ny, _] = grid.dims;
    let layer = nx * ny;
    let sheet_tables = WaveTables::new(&recipe.sheets, grid);
    let hole_tables = WaveTables::new(&recipe.holes, grid);
    let xs = grid.axis_coords(0);
    let ys = grid.axis_coords(1);
    let zs = grid.axis_coords(2);
    let vox = grid.voxel() as f32;
    // The lattice only matters where the skin term does not dominate.
    let active_above = recipe.shell - recipe.blend - vox;

    let mut solid = vec![0f32; grid.len()];
    solid.par_chunks_mut(layer).enumerate().for_each_init(
        || (RowBuffers::new(nx, recipe.sheets.len()), RowBuffers::new(nx, recipe.holes.len())),
        |(b1, b2), (k, plane)| {
            for j in 0..ny {
                let base = (k * ny + j) * nx;
                let flags = &vol.flags[base..base + nx];
                let dist = &vol.dist[base..base + nx];
                let row = &mut plane[j * nx..(j + 1) * nx];
                let active = |i: usize| flags[i] & flag::EXT != 0 && dist[i] > active_above;
                let first = (0..nx).find(|&i| active(i));
                let Some(i0) = first else {
                    for i in 0..nx {
                        row[i] = recipe.shell - dist[i];
                    }
                    continue;
                };
                let i1 = (0..nx).rev().find(|&i| active(i)).unwrap() + 1;
                let len = i1 - i0;
                sheet_tables.row_value_grad(j, k, i0, len, b1);
                hole_tables.row_value(j, k, i0, len, b2);
                for i in 0..nx {
                    let skin = recipe.shell - dist[i];
                    row[i] = if i >= i0 && i < i1 && active(i) {
                        let n = i - i0;
                        let grad_sq = b1.gx[n] * b1.gx[n] + b1.gy[n] * b1.gy[n] + b1.gz[n] * b1.gz[n];
                        let level = recipe.level(recipe.rho(dist[i], [xs[i], ys[j], zs[k]]));
                        let lat = recipe.lattice(level, b1.value[n], grad_sq, b2.value[n]);
                        smooth_max(lat, skin, recipe.blend)
                    } else {
                        skin
                    };
                }
            }
        },
    );
    smooth3(&mut solid, grid.dims);
    solid
}

/// Solid fraction of the lattice region: core voxels, leaving out the ones
/// `exclude` marks (material added on purpose, such as vaults).
pub fn core_density(vol: &Volume, solid: &[f32], exclude: Option<&[bool]>) -> f64 {
    let (solid_count, total) = solid
        .par_iter()
        .zip(vol.flags.par_iter())
        .enumerate()
        .filter(|&(idx, (_, &f))| f & (flag::INSIDE | flag::SHELL) == flag::INSIDE && !exclude.is_some_and(|e| e[idx]))
        .map(|(_, (&s, _))| ((s > 0.0) as usize, 1usize))
        .reduce(|| (0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    if total == 0 { 0.0 } else { solid_count as f64 / total as f64 }
}

/// What the reinforced spheres did.
///
/// A sphere smaller than a cell can fall in a pore or on a wall, so its own
/// density says little; what shows the reinforcement is the comparison with
/// the same spot without it (`unreinforced`).
#[derive(Clone, Debug, Serialize)]
pub struct LoadZones {
    /// Solid fraction of the lattice region inside the spheres.
    pub inside: f64,
    /// The same, estimated without the reinforcement: the same field and
    /// calibration with the load term left out, before the printability
    /// passes.
    pub unreinforced: f64,
    /// Solid fraction of the lattice region outside every sphere.
    pub outside: f64,
    /// Lattice-region voxels each sphere covers, in the order given. A sphere
    /// that covers none is outside the part, or entirely in its skin.
    pub covered: Vec<usize>,
}

impl Recipe {
    /// Whether a lattice voxel is solid by the raw field, with or without the
    /// load term (no skin merge: meant for the lattice region).
    fn raw_solid(&self, dist: f32, p: [f64; 3], with_loads: bool) -> bool {
        let pf = [p[0] as f32, p[1] as f32, p[2] as f32];
        let rho = if with_loads {
            self.rho(dist, pf)
        } else {
            1.0 + self.skin_boost * (-(dist - self.shell).max(0.0) * self.inv_skin_scale).exp()
        };
        let (g1, grad) = self.sheets.eval(p);
        let (g2, _) = self.holes.eval(p);
        let grad_sq = grad[0] * grad[0] + grad[1] * grad[1] + grad[2] * grad[2];
        self.lattice(self.level(rho), g1, grad_sq, g2) > 0.0
    }
}

/// Densities inside and outside the load spheres, or `None` without loads.
pub fn load_zones(vol: &Volume, solid: &[f32], recipe: &Recipe, loads: &[Load]) -> Option<LoadZones> {
    if loads.is_empty() {
        return None;
    }
    let g = &vol.grid;
    let [nx, ny, nz] = g.dims;
    let lattice = |idx: usize| vol.flags[idx] & (flag::INSIDE | flag::SHELL) == flag::INSIDE;
    let in_sphere =
        |p: [f64; 3], l: &Load| (0..3).map(|a| (p[a] - l.center[a]).powi(2)).sum::<f64>() <= l.radius * l.radius;

    let (si, ti, so, to) = (0..solid.len())
        .into_par_iter()
        .filter(|&idx| lattice(idx))
        .map(|idx| {
            let p = [g.x(idx % nx), g.y((idx / nx) % ny), g.z(idx / (nx * ny))];
            let s = (solid[idx] > 0.0) as usize;
            if loads.iter().any(|l| in_sphere(p, l)) { (s, 1, 0, 0) } else { (0, 0, s, 1) }
        })
        .reduce(|| (0, 0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2, a.3 + b.3));

    // Per sphere, only the voxels of its bounding box.
    let range = |axis: usize, c: f64, r: f64, n: usize| {
        let lo = ((c - r - g.origin[axis]) / g.spacing[axis]).floor().max(0.0) as usize;
        let hi = (((c + r - g.origin[axis]) / g.spacing[axis]).ceil().max(0.0) as usize + 1).min(n);
        lo.min(n)..hi
    };
    // Per sphere, only the voxels of its bounding box. A voxel shared by
    // several spheres counts once towards the unreinforced estimate.
    let mut covered = Vec::with_capacity(loads.len());
    let (mut raw_solid, mut raw_total) = (0usize, 0usize);
    for (n, l) in loads.iter().enumerate() {
        let (count, s, t) = range(2, l.center[2], l.radius, nz)
            .into_par_iter()
            .map(|k| {
                let (mut count, mut s, mut t) = (0, 0, 0);
                for j in range(1, l.center[1], l.radius, ny) {
                    for i in range(0, l.center[0], l.radius, nx) {
                        let idx = i + nx * (j + ny * k);
                        let p = [g.x(i), g.y(j), g.z(k)];
                        if !lattice(idx) || !in_sphere(p, l) {
                            continue;
                        }
                        count += 1;
                        if loads[..n].iter().any(|earlier| in_sphere(p, earlier)) {
                            continue;
                        }
                        t += 1;
                        s += recipe.raw_solid(vol.dist[idx], p, false) as usize;
                    }
                }
                (count, s, t)
            })
            .reduce(|| (0, 0, 0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
        covered.push(count);
        raw_solid += s;
        raw_total += t;
    }

    let fraction = |s: usize, t: usize| if t == 0 { 0.0 } else { s as f64 / t as f64 };
    Some(LoadZones {
        inside: fraction(si, ti),
        unreinforced: fraction(raw_solid, raw_total),
        outside: fraction(so, to),
        covered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::mesh::box_mesh;

    fn setup(density: f64) -> (Volume, Params) {
        let mesh = box_mesh([0.0; 3], [30.0; 3]);
        let p = Params { density, cell: 10.0, shell: 1.5, voxel: 0.5, ..Params::default() };
        let grid = Grid::enclosing([0.0; 3], [30.0; 3], 0.5, 2.0);
        (Volume::build(&mesh, grid, &p), p)
    }

    #[test]
    fn calibration_hits_target_density() {
        for target in [0.12, 0.2, 0.35] {
            let (vol, p) = setup(target);
            let (recipe, cal) = calibrate(&vol, &p, target);
            assert!((cal.predicted_density - target).abs() < 0.01, "{target}: {cal:?}");
            let solid = evaluate(&vol, &recipe);
            // The evaluated field adds skin fillets and smoothing, so allow slack.
            let measured = core_density(&vol, &solid, None);
            assert!((measured - target).abs() < 0.08, "target {target}, measured {measured}");
        }
    }

    #[test]
    fn skin_is_solid_and_outside_is_positive() {
        let (vol, p) = setup(0.16);
        let (recipe, _) = calibrate(&vol, &p, 0.16);
        let solid = evaluate(&vol, &recipe);
        for (idx, &f) in vol.flags.iter().enumerate() {
            if f & flag::SHELL != 0 {
                assert!(solid[idx] > 0.0, "hole in the skin at {idx}");
            }
            if f & flag::EXT == 0 {
                assert!(solid[idx] > 0.0);
            }
        }
    }

    #[test]
    fn same_seed_same_field() {
        let (vol, p) = setup(0.16);
        let a = evaluate(&vol, &calibrate(&vol, &p, 0.16).0);
        let b = evaluate(&vol, &calibrate(&vol, &p, 0.16).0);
        assert_eq!(a, b);
        let other = Params { seed: 8, ..p.clone() };
        let c = evaluate(&vol, &calibrate(&vol, &other, 0.16).0);
        assert_ne!(a, c);
    }

    #[test]
    fn less_perforation_means_thinner_walls_for_the_same_density() {
        let (vol, p) = setup(0.16);
        let rods = Params { perforation: [0.5, 0.4], ..p.clone() };
        let plates = Params { perforation: [0.1, 0.05], ..p };
        let (_, a) = calibrate(&vol, &rods, 0.16);
        let (_, b) = calibrate(&vol, &plates, 0.16);
        assert!((a.predicted_density - 0.16).abs() < 0.01 && (b.predicted_density - 0.16).abs() < 0.01);
        assert!(b.mean_wall < a.mean_wall, "plates {} vs rods {}", b.mean_wall, a.mean_wall);
    }

    #[test]
    fn load_zones_measure_the_reinforcement_and_spot_spheres_that_miss() {
        let (vol, p) = setup(0.16);
        let inner = Load { center: [15.0, 15.0, 15.0], radius: 5.0 };
        let away = Load { center: [200.0, 15.0, 15.0], radius: 5.0 };
        let p = Params { loads: vec![inner, away], load_boost: 2.0, ..p };
        let recipe = calibrate(&vol, &p, 0.16).0;
        let solid = evaluate(&vol, &recipe);
        assert!(load_zones(&vol, &solid, &recipe, &[]).is_none());
        let zones = load_zones(&vol, &solid, &recipe, &p.loads).unwrap();
        // The sphere is smaller than a cell, so compare it with itself
        // without the reinforcement rather than with the rest of the part.
        assert!(zones.inside > 1.5 * zones.unreinforced, "{zones:?}");
        assert!(zones.covered[0] > 100, "{zones:?}");
        assert_eq!(zones.covered[1], 0, "a sphere outside the part covers nothing");
        // Without reinforcement the estimate matches what is there.
        let none = Params { load_boost: 0.0, ..p };
        let recipe = calibrate(&vol, &none, 0.16).0;
        let solid = evaluate(&vol, &recipe);
        let zones = load_zones(&vol, &solid, &recipe, &none.loads).unwrap();
        // An estimate on a sphere smaller than a cell: a few points of slack.
        assert!((zones.inside - zones.unreinforced).abs() < 0.04, "{zones:?}");
    }

    #[test]
    fn load_regions_get_denser_and_load_boost_controls_it() {
        let (vol, p) = setup(0.16);
        let load = crate::params::Load { center: [15.0, 15.0, 15.0], radius: 4.0 };
        let density_near = |boost: f64| {
            let p = Params { loads: vec![load], load_boost: boost, ..p.clone() };
            let solid = evaluate(&vol, &calibrate(&vol, &p, 0.16).0);
            let g = &vol.grid;
            let (mut n, mut m) = (0usize, 0usize);
            for idx in 0..solid.len() {
                let (i, j, k) = (idx % g.nx(), (idx / g.nx()) % g.ny(), idx / g.layer_len());
                let d = ((g.x(i) - 15.0).powi(2) + (g.y(j) - 15.0).powi(2) + (g.z(k) - 15.0).powi(2)).sqrt();
                if d < 4.0 && vol.is_core(idx) {
                    m += 1;
                    n += (solid[idx] > 0.0) as usize;
                }
            }
            n as f64 / m as f64
        };
        let (off, on) = (density_near(0.0), density_near(2.0));
        assert!(on > off + 0.05, "with boost {on}, without {off}");
    }

    #[test]
    fn walls_respect_minimum_thickness() {
        let (vol, mut p) = setup(0.05);
        p.min_wall = 1.5;
        let (_, cal) = calibrate(&vol, &p, 0.05);
        assert!(cal.min_wall >= 1.5 - 1e-3);
        assert!(cal.wall_clamped);
    }
}
