//! Voxel classification of the input model: inside / skin / core, distance to
//! the skinned surface, and clearance below inner ceilings.

use crate::edt;
use crate::grid::{Grid, flag};
use crate::mesh::TriMesh;
use crate::params::{Face, Params};
use crate::voxelize::voxelize;
use rayon::prelude::*;

/// "No ceiling above this voxel" marker in [`Volume::ceiling`].
pub const NO_CEILING: u8 = 255;

pub struct Volume {
    pub grid: Grid,
    /// [`flag`] bits per voxel.
    pub flags: Vec<u8>,
    /// Distance (mm) from each voxel to the nearest *closed* (skinned) part of
    /// the model surface; 0 outside the model.
    pub dist: Vec<f32>,
    /// For core voxels lying under an inner ceiling that overhangs too much
    /// to print unaided: clearance to that ceiling, straight up, in voxels
    /// (saturating at 254). [`NO_CEILING`] everywhere else.
    pub ceiling: Vec<u8>,
    /// How flat that ceiling is, 0..=255: 255 for a horizontal roof, which the
    /// first skin layer can cross as a bridge; 0 for a sloping one, where each
    /// skin layer only sticks out past the previous one and nothing can be
    /// bridged. Meaningful where `ceiling` is set.
    pub ceiling_flatness: Vec<u8>,
    /// First Z layer containing model voxels: it rests on the build plate.
    pub bed_layer: usize,
    pub inside_voxels: usize,
    pub core_voxels: usize,
}

impl Volume {
    pub fn build(mesh: &TriMesh, grid: Grid, p: &Params) -> Self {
        let vox = grid.voxel() as f32;
        let mut flags = voxelize(mesh, &grid);
        // voxelize() writes 1 = INSIDE; closed faces make EXT identical.
        flags.par_iter_mut().for_each(|f| {
            if *f != 0 {
                *f = flag::INSIDE | flag::EXT;
            }
        });
        for face in &p.open {
            mark_open_region(&mut flags, &grid, *face);
        }

        // Voxel-centre distances overestimate the distance to the true surface
        // by about half a voxel; remove that bias so `shell` is honest.
        let mut dist = edt::distance_to(grid.dims, |i| flags[i] & flag::EXT == 0);
        dist.par_iter_mut().for_each(|d| *d = (*d - 0.5).max(0.0) * vox);

        let shell = p.shell as f32;
        flags.par_iter_mut().zip(dist.par_iter()).for_each(|(f, &d)| {
            if *f & flag::INSIDE != 0 && d <= shell {
                *f |= flag::SHELL;
            }
        });
        if !p.open.is_empty() {
            // Skin within a skin's width (and a voxel) of the opened region.
            let opened = edt::distance_to(grid.dims, |i| flags[i] & (flag::INSIDE | flag::EXT) == flag::EXT);
            let reach = shell / vox + 1.0;
            flags.par_iter_mut().zip(opened.par_iter()).for_each(|(f, &d)| {
                if *f & flag::SHELL != 0 && d <= reach {
                    *f |= flag::RIM;
                }
            });
        }

        let layer = grid.layer_len();
        let counts: Vec<(usize, usize)> = flags
            .par_chunks(layer)
            .map(|plane| {
                let inside = plane.iter().filter(|&&f| f & flag::INSIDE != 0).count();
                let shell = plane.iter().filter(|&&f| f & flag::SHELL != 0).count();
                (inside, inside - shell)
            })
            .collect();
        let bed_layer = counts.iter().position(|c| c.0 > 0).unwrap_or(0);
        let inside_voxels = counts.iter().map(|c| c.0).sum();
        let core_voxels = counts.iter().map(|c| c.1).sum();

        let (ceiling, ceiling_flatness) = if p.vaults {
            ceiling_clearance(&grid, &flags, &dist, p.max_overhang_deg)
        } else {
            (vec![NO_CEILING; grid.len()], vec![0; grid.len()])
        };

        Self { grid, flags, dist, ceiling, ceiling_flatness, bed_layer, inside_voxels, core_voxels }
    }

    #[inline]
    pub fn is_core(&self, idx: usize) -> bool {
        self.flags[idx] & (flag::INSIDE | flag::SHELL) == flag::INSIDE
    }
}

/// Marks as EXT the outside voxels lying beyond the model along `face`:
/// the skin is not generated on surfaces that look that way.
fn mark_open_region(flags: &mut [u8], grid: &Grid, face: Face) {
    let [nx, ny, nz] = grid.dims;
    let layer = nx * ny;
    match face.axis {
        0 => flags.par_chunks_mut(nx).for_each(|row| sweep(row.iter_mut(), face.positive)),
        1 => flags.par_chunks_mut(layer).for_each(|plane| {
            for i in 0..nx {
                sweep(plane.iter_mut().skip(i).step_by(nx), face.positive);
            }
        }),
        _ => {
            let mut seen = vec![false; layer];
            let mut visit = |k: usize| {
                let plane = &mut flags[k * layer..(k + 1) * layer];
                for (f, s) in plane.iter_mut().zip(seen.iter_mut()) {
                    if *f & flag::INSIDE != 0 {
                        *s = true;
                    } else if *s {
                        *f |= flag::EXT;
                    }
                }
            };
            if face.positive {
                (0..nz).for_each(&mut visit);
            } else {
                (0..nz).rev().for_each(&mut visit);
            }
        }
    }
}

fn sweep<'a, I>(line: I, positive: bool)
where
    I: DoubleEndedIterator<Item = &'a mut u8>,
{
    let mut seen = false;
    let mut step = |f: &mut u8| {
        if *f & flag::INSIDE != 0 {
            seen = true;
        } else if seen {
            *f |= flag::EXT;
        }
    };
    if positive {
        line.for_each(&mut step);
    } else {
        line.rev().for_each(&mut step);
    }
}

/// For every core voxel, how far above it the skin starts (straight up),
/// provided that piece of ceiling is too flat to print without help. Steep
/// inner walls are left alone.
fn ceiling_clearance(grid: &Grid, flags: &[u8], dist: &[f32], max_overhang_deg: f64) -> (Vec<u8>, Vec<u8>) {
    let [nx, ny, nz] = grid.dims;
    let layer = nx * ny;
    let mut out = vec![NO_CEILING; grid.len()];
    let mut flatness = vec![0u8; grid.len()];
    // Horizontal share of the surface normal: 0 for a flat roof. Roofs within
    // about 5 degrees of horizontal count as fully flat, beyond 20 as sloping.
    let (flat_below, sloping_above) = (5f32.to_radians().sin(), 20f32.to_radians().sin());
    // Flag ceilings generously: the vault itself fades out where the roof is
    // steep enough, and the slope estimated here from a voxelised distance
    // field is noisy.
    let threshold = (max_overhang_deg - 20.0).max(5.0).to_radians().sin() as f32;

    // Per column: layer index of the skin voxel met most recently while
    // walking down, and whether that ceiling needs a vault.
    let mut last = vec![-1i32; layer];
    let mut needs = vec![false; layer];
    let mut flat = vec![0u8; layer];
    for k in (0..nz).rev() {
        let base = k * layer;
        for j in 0..ny {
            for i in 0..nx {
                let p = j * nx + i;
                let idx = base + p;
                let f = flags[idx];
                if f & flag::SHELL != 0 {
                    last[p] = k as i32;
                } else if f & flag::INSIDE == 0 {
                    last[p] = -1;
                } else if last[p] >= 0 {
                    let gap = last[p] - k as i32;
                    if gap == 1 {
                        // First core voxel under the skin: measure the slope.
                        // The distance grows inwards, so a ceiling has a
                        // downward gradient; -g_z/|g| = sin(overhang angle).
                        let g = gradient(dist, grid, i, j, k);
                        let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
                        needs[p] = len > 0.0 && -g[2] / len > threshold;
                        if needs[p] {
                            let horizontal = (g[0] * g[0] + g[1] * g[1]).sqrt() / len;
                            let sloping = crate::mathx::smoothstep(flat_below, sloping_above, horizontal);
                            flat[p] = ((1.0 - sloping) * 255.0).round() as u8;
                        }
                    }
                    if needs[p] {
                        out[idx] = gap.min(254) as u8;
                        flatness[idx] = flat[p];
                    }
                }
            }
        }
    }
    (out, flatness)
}

fn gradient(field: &[f32], grid: &Grid, i: usize, j: usize, k: usize) -> [f32; 3] {
    let [nx, ny, nz] = grid.dims;
    let at = |i: usize, j: usize, k: usize| field[(k * ny + j) * nx + i];
    [
        at((i + 1).min(nx - 1), j, k) - at(i.saturating_sub(1), j, k),
        at(i, (j + 1).min(ny - 1), k) - at(i, j.saturating_sub(1), k),
        at(i, j, (k + 1).min(nz - 1)) - at(i, j, k.saturating_sub(1)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::box_mesh;

    fn cube_volume(open: Vec<Face>) -> Volume {
        let mesh = box_mesh([0.0; 3], [12.0; 3]);
        let p = Params { open, shell: 1.5, cell: 8.0, ..Params::default() };
        let grid = Grid::enclosing([0.0; 3], [12.0; 3], 0.5, 2.0);
        Volume::build(&mesh, grid, &p)
    }

    #[test]
    fn closed_cube_has_skin_all_around() {
        let v = cube_volume(vec![]);
        let g = &v.grid;
        let centre = g.nearest([6.0, 6.0, 6.0]).unwrap();
        let c = g.index(centre[0], centre[1], centre[2]);
        assert!(v.is_core(c));
        assert!((v.dist[c] - 6.0).abs() < 0.6);
        for probe in [[6.0, 6.0, 0.4], [6.0, 6.0, 11.6], [0.4, 6.0, 6.0], [6.0, 11.6, 6.0]] {
            let n = g.nearest(probe).unwrap();
            let f = v.flags[g.index(n[0], n[1], n[2])];
            assert!(f & flag::SHELL != 0, "no skin at {probe:?}");
        }
        assert!(v.core_voxels > 0 && v.core_voxels < v.inside_voxels);
    }

    #[test]
    fn open_top_has_no_skin_there() {
        let v = cube_volume(vec![Face { axis: 2, positive: true }]);
        let g = &v.grid;
        let n = g.nearest([6.0, 6.0, 11.6]).unwrap();
        let f = v.flags[g.index(n[0], n[1], n[2])];
        assert!(f & flag::INSIDE != 0 && f & flag::SHELL == 0);
        // The bottom still has skin.
        let n = g.nearest([6.0, 6.0, 0.4]).unwrap();
        assert!(v.flags[g.index(n[0], n[1], n[2])] & flag::SHELL != 0);
    }

    #[test]
    fn ceiling_clearance_is_set_under_the_roof_only() {
        let v = cube_volume(vec![]);
        let g = &v.grid;
        // Just under the top skin (skin spans z in 10.5..12).
        let n = g.nearest([6.0, 6.0, 10.0]).unwrap();
        let under_roof = v.ceiling[g.index(n[0], n[1], n[2])];
        assert_eq!(under_roof, 1);
        assert_eq!(v.ceiling_flatness[g.index(n[0], n[1], n[2])], 255);
        // Further down the clearance grows by one per voxel (0.5 mm).
        let n = g.nearest([6.0, 6.0, 4.0]).unwrap();
        assert_eq!(v.ceiling[g.index(n[0], n[1], n[2])], 13);
        // Open top: no ceiling at all.
        let open = cube_volume(vec![Face { axis: 2, positive: true }]);
        assert!(open.ceiling.iter().all(|&c| c == NO_CEILING));
    }
}
