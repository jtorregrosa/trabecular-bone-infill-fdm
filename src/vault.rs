//! Vaults: closing the pores under inner ceilings so the skin above them can
//! be printed.
//!
//! A closed part has a roof, and the first layer of that roof is laid over
//! whatever the lattice offers. Left alone, it would have to span whole pores
//! or, under a sloping roof, stick out further at every layer.
//!
//! The rule that makes a cavity printable is about its *empty* space: going
//! down from a roof, a pore may only widen by `tan(overhang)` per layer.
//! Empty space therefore has to descend from somewhere, and this pass decides
//! from where. It sweeps the model from the top down keeping, for the current
//! layer, the signed distance `r` to the region that is allowed to be empty:
//!
//! 1. each layer down, that region grows by `tan(overhang)` (`r` decreases);
//! 2. skin and the outside of the model are roof (`r` is reset to their
//!    depth), and so is every voxel this pass has filled;
//! 3. where a pore is found deep under a roof with no empty space reaching it
//!    from above, it is *seeded*: a slot is opened along the pore's medial
//!    axis. Under a flat roof the slot is as wide as a bridge the first skin
//!    layer can cross; under a sloping roof, where nothing can be bridged, it
//!    is a hairline;
//! 4. whatever part of a pore is still under roof is filled.
//!
//! The filled material is a pointed (gothic) vault springing from the walls
//! of each pore, corbelled out layer by layer so each layer rests on the one
//! below. Under a dome it is the wedge between the dome and a cone of the
//! printable angle, and it vanishes by itself where the dome is steep enough.
//! Small pores get shallow vaults, large pores tall ones.

use crate::edt::{self, FeatureScratch};
use crate::grid::flag;
use crate::params::Params;
use crate::volume::Volume;

/// A pixel is on the medial axis of a pore when it and a neighbour look at
/// wall points that subtend at least this angle: 180 degrees between two
/// facing walls, 90 on the bisector of a square corner. The threshold keeps
/// the axis of channels and rounded pores and drops the spurs that run into
/// shallow corners and wall roughness.
const AXIS_MIN_ANGLE_DEG: f32 = 80.0;
/// The two wall points must also be genuinely different ones (pixels apart).
const AXIS_MIN_SEPARATION: f32 = 2.0;
/// Right next to a wall every notch of its voxel staircase passes the angle
/// test, so axis pixels must be at least this far (pixels) from the walls.
/// Pores too narrow to have an axis open through their wider neighbours.
const AXIS_MIN_WALL_DISTANCE: f32 = 2.0;

/// A pore is seeded where it lies more than this many voxels under roof.
const SEED_DEPTH: f32 = 1.0;

/// "Roof as far as the eye can see" and "open sky".
const ROOF: f32 = 1.0e6;
const OPEN: f32 = -1.0e3;

/// Adds vault material to `solid`. Returns a mask of the voxels that became
/// solid and their count.
pub fn build_vaults(vol: &Volume, solid: &mut [f32], p: &Params) -> (Vec<bool>, usize) {
    let grid = &vol.grid;
    let [nx, ny, nz] = grid.dims;
    let layer = nx * ny;
    let vox = grid.voxel() as f32;
    let mut mask = vec![false; grid.len()];
    if !p.vaults {
        return (mask, 0);
    }
    // A little steeper than the limit, so the overhang filter that runs
    // afterwards does not shave the vault it has just been given.
    let slope = (p.max_overhang_deg - 5.0).max(5.0).to_radians().tan() as f32;
    let half_slot = 0.5 * (p.vault_slot * p.max_bridge) as f32 / vox;

    let mut r = vec![ROOF; layer];
    let mut roof = vec![true; layer];
    let mut void = vec![false; layer];
    let mut axis = vec![false; layer];
    let mut dist = vec![0f32; layer];
    let mut site = vec![0u32; layer];
    let mut scratch = FeatureScratch::new(nx, ny);
    let mut added = 0usize;

    for k in (0..nz).rev() {
        let flags = &vol.flags[k * layer..(k + 1) * layer];
        let plane = &mut solid[k * layer..(k + 1) * layer];

        // 1 + 2: the allowed region grows; skin and outside are roof.
        let mut any_core = false;
        let mut any_void = false;
        for i in 0..layer {
            let f = flags[i];
            if f & flag::INSIDE == 0 {
                // Outside the model: open if a face was opened this way.
                roof[i] = f & flag::EXT == 0;
                r[i] = if roof[i] { ROOF } else { OPEN };
                void[i] = false;
            } else if f & flag::SHELL != 0 {
                roof[i] = true;
                r[i] = ROOF;
                void[i] = false;
            } else {
                roof[i] = false;
                if r[i] < 0.5 * ROOF {
                    r[i] = (r[i] - slope).max(OPEN);
                }
                void[i] = plane[i] <= 0.0;
                any_core = true;
                any_void |= void[i];
            }
        }
        if !any_core {
            continue;
        }
        relax(&mut r, &roof, nx, ny);
        if !any_void {
            continue;
        }

        // 3: seed pores that lie under roof.
        if (0..layer).any(|i| void[i] && r[i] > SEED_DEPTH) {
            let void_ref: &[bool] = &void;
            edt::feature_transform_2d(nx, ny, &mut dist, &mut site, |i| !void_ref[i], &mut scratch);
            mark_axis(&mut axis, void_ref, &dist, &site, nx, ny);
            let mut any_seed = false;
            for i in 0..layer {
                axis[i] = axis[i] && r[i] > SEED_DEPTH;
                any_seed |= axis[i];
            }
            if any_seed {
                let axis_ref: &[bool] = &axis;
                edt::feature_transform_2d(nx, ny, &mut dist, &mut site, |i| axis_ref[i], &mut scratch);
                let ceiling = &vol.ceiling[k * layer..(k + 1) * layer];
                let flatness = &vol.ceiling_flatness[k * layer..(k + 1) * layer];
                for i in 0..layer {
                    if !void[i] || site[i] == u32::MAX {
                        continue;
                    }
                    // A bridge-wide slot only directly under a flat piece of
                    // skin; a hairline anywhere else.
                    let seed = site[i] as usize;
                    let slot = if ceiling[seed] == 1 { half_slot * flatness[seed] as f32 / 255.0 } else { 0.0 };
                    r[i] = r[i].min(dist[i] - slot - 0.5);
                }
            }
        }

        // 4: fill what is still under roof. The value is distance-like, so
        // the vault surface comes out smooth.
        let mask_plane = &mut mask[k * layer..(k + 1) * layer];
        for i in 0..layer {
            if !void[i] {
                continue;
            }
            let value = (r[i] * vox).min(2.0 * vox);
            if value > plane[i] {
                if value > 0.0 {
                    mask_plane[i] = true;
                    added += 1;
                }
                plane[i] = value;
            }
        }
    }
    (mask, added)
}

/// Restores `r` as a distance after roof pixels were reset: two chamfer
/// passes of `r(p) = min(r(p), r(q) + |p - q|)`.
///
/// Open space next to a roof does not reach *into* it at once: seen from a
/// roof pixel a non-roof neighbour counts as distance 0 at best, so the depth
/// of a roof pixel is at least its distance to the roof's edge.
fn relax(r: &mut [f32], roof: &[bool], nx: usize, ny: usize) {
    const D1: f32 = 1.0;
    const D2: f32 = std::f32::consts::SQRT_2;
    let mut visit = |i: usize, j: usize, neighbours: &[(i32, i32, f32)]| {
        let p = j * nx + i;
        let mut best = r[p];
        for &(di, dj, d) in neighbours {
            let (ii, jj) = (i as i32 + di, j as i32 + dj);
            if ii < 0 || jj < 0 || ii >= nx as i32 || jj >= ny as i32 {
                continue;
            }
            let q = jj as usize * nx + ii as usize;
            let from = if roof[p] && !roof[q] { r[q].max(0.0) } else { r[q] };
            best = best.min(from + d);
        }
        r[p] = best;
    };
    let forward = [(-1, 0, D1), (-1, -1, D2), (0, -1, D1), (1, -1, D2)];
    let backward = [(1, 0, D1), (1, 1, D2), (0, 1, D1), (-1, 1, D2)];
    for j in 0..ny {
        for i in 0..nx {
            visit(i, j, &forward);
        }
    }
    for j in (0..ny).rev() {
        for i in (0..nx).rev() {
            visit(i, j, &backward);
        }
    }
}

/// Marks the medial axis of the empty region.
///
/// `site` holds the nearest wall pixel of each empty pixel. The axis runs
/// where neighbouring pixels are nearest to clearly different walls. Testing
/// the angle those wall points subtend, rather than how far apart they are,
/// makes the test independent of the size of the pore: a fixed distance would
/// flag a whole blob around the centre of any large rounded pore. (Ridges of
/// the distance map would also do in principle, but the staircase of a
/// diagonal wall is full of false ones.)
fn mark_axis(axis: &mut [bool], void: &[bool], dist: &[f32], site: &[u32], nx: usize, ny: usize) {
    let max_cos = AXIS_MIN_ANGLE_DEG.to_radians().cos();
    let straddles = |p: usize, s: u32, t: u32| {
        if s == u32::MAX || t == u32::MAX || s == t {
            return false;
        }
        let xy = |i: usize| ((i % nx) as f32, (i / nx) as f32);
        let (px, py) = xy(p);
        let (sx, sy) = xy(s as usize);
        let (tx, ty) = xy(t as usize);
        if (sx - tx).hypot(sy - ty) < AXIS_MIN_SEPARATION {
            return false;
        }
        let (ax, ay) = (sx - px, sy - py);
        let (bx, by) = (tx - px, ty - py);
        let norm = ax.hypot(ay) * bx.hypot(by);
        norm > 0.0 && (ax * bx + ay * by) / norm <= max_cos
    };
    axis.fill(false);
    for j in 0..ny {
        for i in 0..nx {
            let idx = j * nx + i;
            if !void[idx] {
                continue;
            }
            // Of the two pixels astride the axis, keep the one farther from
            // the walls.
            for other in [idx + 1, idx + nx] {
                let in_grid = if other == idx + 1 { i + 1 < nx } else { j + 1 < ny };
                if in_grid && void[other] && straddles(idx, site[idx], site[other]) {
                    let pick = if dist[idx] >= dist[other] { idx } else { other };
                    if dist[pick] >= AXIS_MIN_WALL_DISTANCE {
                        axis[pick] = true;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;
    use crate::volume::NO_CEILING;

    /// A hollow box: one-voxel skin all around, empty core.
    fn hollow_box(n: usize) -> (Volume, Vec<f32>) {
        let grid = Grid { origin: [0.0; 3], spacing: [1.0; 3], dims: [n, n, n] };
        let mut flags = vec![0u8; grid.len()];
        let mut ceiling = vec![NO_CEILING; grid.len()];
        let flatness = vec![255u8; grid.len()];
        for k in 1..n - 1 {
            for j in 1..n - 1 {
                for i in 1..n - 1 {
                    let edge = |v: usize| v == 1 || v == n - 2;
                    let mut f = flag::INSIDE | flag::EXT;
                    if edge(i) || edge(j) || edge(k) {
                        f |= flag::SHELL;
                    } else {
                        ceiling[grid.index(i, j, k)] = (n - 2 - k) as u8;
                    }
                    flags[grid.index(i, j, k)] = f;
                }
            }
        }
        let solid: Vec<f32> = flags.iter().map(|&f| if f & flag::SHELL != 0 { 1.0 } else { -1.0 }).collect();
        let len = grid.len();
        let vol = Volume {
            grid,
            flags,
            dist: vec![0.0; len],
            ceiling,
            ceiling_flatness: flatness,
            bed_layer: 1,
            inside_voxels: 0,
            core_voxels: 0,
        };
        (vol, solid)
    }

    #[test]
    fn empty_box_gets_a_vault_that_narrows_towards_the_roof() {
        let n = 44;
        let (vol, mut solid) = hollow_box(n);
        let p = Params { max_bridge: 6.0, max_overhang_deg: 45.0, ..Params::default() };
        let (_, added) = build_vaults(&vol, &mut solid, &p);
        assert!(added > 0);
        let g = &vol.grid;
        let open = |i: usize, j: usize, k: usize| solid[g.index(i, j, k)] <= 0.0;
        let open_count =
            |k: usize| (2..n - 2).flat_map(|j| (2..n - 2).map(move |i| (i, j))).filter(|&(i, j)| open(i, j, k)).count();

        // Just under the roof only slots along the skeleton of the square
        // (its two diagonals) stay open, each about one bridge wide.
        let top = n - 3;
        let centre = (n as f32 - 1.0) / 2.0;
        let reach = 0.5 * p.vault_slot as f32 * 6.0 + (45.0f32 - 5.0).to_radians().tan() + 1.0;
        for j in 2..n - 2 {
            for i in 2..n - 2 {
                if open(i, j, top) {
                    let (x, y) = (i as f32 - centre, j as f32 - centre);
                    let to_diagonal = (x - y).abs().min((x + y).abs()) / std::f32::consts::SQRT_2;
                    assert!(to_diagonal <= reach, "open voxel {i},{j} is {to_diagonal} from the skeleton");
                }
            }
        }
        assert!(open_count(top) > 0);
        // The opening grows going down until the box is fully open again.
        let mut prev = open_count(top);
        for k in (2..top).rev() {
            let c = open_count(k);
            assert!(c >= prev, "opening shrinks going down at k = {k}");
            prev = c;
        }
        assert_eq!(open_count(2), (n - 4) * (n - 4));
    }

    #[test]
    fn axis_of_a_round_pore_is_its_centre_not_a_blob() {
        let n = 61usize;
        let c = 30.0f32;
        let void: Vec<bool> = (0..n * n).map(|i| ((i % n) as f32 - c).hypot((i / n) as f32 - c) < 26.0).collect();
        let mut dist = vec![0f32; n * n];
        let mut site = vec![0u32; n * n];
        edt::feature_transform_2d(n, n, &mut dist, &mut site, |i| !void[i], &mut FeatureScratch::new(n, n));
        let mut axis = vec![false; n * n];
        mark_axis(&mut axis, &void, &dist, &site, n, n);
        let marked: Vec<usize> = (0..n * n).filter(|&i| axis[i]).collect();
        assert!(!marked.is_empty());
        for i in marked {
            let d = ((i % n) as f32 - c).hypot((i / n) as f32 - c);
            assert!(d <= 3.0, "axis pixel {d} px away from the centre of a 26 px disc");
        }
    }

    #[test]
    fn axis_of_a_channel_is_its_centre_line() {
        let (nx, ny) = (40usize, 15usize);
        let void: Vec<bool> = (0..nx * ny).map(|i| (4..11).contains(&(i / nx))).collect();
        let mut dist = vec![0f32; nx * ny];
        let mut site = vec![0u32; nx * ny];
        edt::feature_transform_2d(nx, ny, &mut dist, &mut site, |i| !void[i], &mut FeatureScratch::new(nx, ny));
        let mut axis = vec![false; nx * ny];
        mark_axis(&mut axis, &void, &dist, &site, nx, ny);
        for i in 0..nx * ny {
            assert_eq!(axis[i], i / nx == 7, "pixel {},{}", i % nx, i / nx);
        }
    }

    #[test]
    fn disabled_vaults_change_nothing() {
        let (vol, mut solid) = hollow_box(20);
        let before = solid.clone();
        let p = Params { vaults: false, ..Params::default() };
        let (_, added) = build_vaults(&vol, &mut solid, &p);
        assert_eq!(added, 0);
        assert_eq!(solid, before);
    }
}
