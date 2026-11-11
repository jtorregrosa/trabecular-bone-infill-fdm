//! Makes the lattice printable without internal supports.
//!
//! The model is walked layer by layer from the build plate. A lattice voxel
//! survives only if the nozzle can actually lay it down:
//!
//! * **Overhang rule** – it lies within `tan(max_overhang)` layer heights of
//!   material kept in the layer below. Applied at every layer this bounds the
//!   slope of all lattice undersides; holes and pores get pointed "gothic"
//!   roofs instead of flat ones.
//! * **Bridge rule** – otherwise, it is part of a band of strands no longer
//!   than `max_bridge`, each resting on supported material at *both* ends
//!   (see [`crate::bridge`]). One-sided cantilevers are never accepted.
//!
//! Everything else is carved away, including whatever only hung from above.
//! The skin is never modified: its outer overhangs are the slicer's concern
//! (external supports), its inner ceilings are carried by [`crate::vault`].
//! The exception is the skin along an opened face ([`flag::RIM`]): what held
//! it up may have been opened away, so it is treated like the lattice.
//!
//! The rule is evaluated on distances, not on a binary mask. Each layer keeps
//! `a(q)`, how far inside the supported material voxel `q` lies (taken from
//! the field, which is a distance to the surface). A voxel `p` of the next
//! layer may then extend to
//!
//! ```text
//! allowance(p) = tan(max_overhang) + max_q ( a(q) - |p - q| )
//! ```
//!
//! and the field is clipped to that allowance. This is the in-plane signed
//! distance to the previous layer's outline grown by one layer's worth of
//! overhang, so the limit is the same in every direction and the cut surface
//! is a smooth plane at the limit angle rather than a voxel staircase.
//!
//! The empty voxel underneath a clipped one is lowered to the matching
//! distance (the same expression without the `tan` term). Without that, the
//! surface between the two layers would be interpolated against whatever the
//! original field held there, and the kept voxel would hang down as a small
//! lip steeper than the limit.

use crate::bridge::{self, BRIDGED, BridgeScratch, Mode, SUPPORTED, UNSUPPORTED, VOID};
use crate::grid::flag;
use crate::params::Params;
use crate::volume::Volume;
use rayon::prelude::*;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct SupportStats {
    /// Lattice voxels removed because nothing could hold them.
    pub removed_voxels: usize,
    /// Lattice voxels kept as bridges.
    pub bridged_voxels: usize,
}

/// "Nothing to stand on here."
const NO_SUPPORT: f32 = -1.0e3;
/// Depth (voxels) beyond which a supported voxel's exact depth is irrelevant.
const DEPTH_CAP: f32 = 1.5;
/// Depth (voxels) the material under a piece must reach somewhere for that
/// piece to count as standing on it rather than on a sliver.
const MIN_ANCHOR_DEPTH: f32 = 0.5;
/// Largest piece (mm2 of one layer, at least two voxels) removed for
/// standing on a sliver only.
/// Small pieces are the tips of struts that would start in mid-air; larger
/// ones are left to the overhang rule, so that removing a tip does not set
/// off a cascade through the lattice above it.
const MAX_LOOSE_TIP: f32 = 0.4;

/// Neighbour offsets within `radius` voxels, nearest first.
fn disc_offsets(radius: f32) -> Vec<(i32, i32, f32)> {
    let r = radius.ceil() as i32;
    let mut out = Vec::new();
    for dj in -r..=r {
        for di in -r..=r {
            if di == 0 && dj == 0 {
                continue;
            }
            let d = ((di * di + dj * dj) as f32).sqrt();
            if d <= radius {
                out.push((di, dj, d));
            }
        }
    }
    out.sort_by(|a, b| a.2.total_cmp(&b.2));
    out
}

/// Carves unsupported lattice out of `solid` in place.
pub fn make_printable(vol: &Volume, solid: &mut [f32], p: &Params) -> SupportStats {
    let grid = &vol.grid;
    let [nx, ny, nz] = grid.dims;
    let layer = nx * ny;
    let vox = grid.voxel() as f32;
    let inv_vox = 1.0 / vox;

    // Sideways reach gained per layer, in voxels (layers are one voxel tall).
    let reach = p.max_overhang_deg.to_radians().tan() as f32;
    let offsets = disc_offsets(reach + DEPTH_CAP + 1.0);
    // Stay a little inside the limits: the part will be judged on its mesh,
    // sliced at another pitch, where spans come out a fraction longer.
    let bridge_voxels = (0.9 * p.max_bridge as f32 / vox - 0.5).max(0.0);
    let band_voxels = (p.min_bridge_width as f32 / vox).max(2.0) + 1.0;

    // Depth inside the supported material of the layer below, in voxels.
    let mut below = vec![NO_SUPPORT; layer];
    let mut state = vec![VOID; layer];
    let mut cut = vec![0f32; layer];
    // In-plane signed distance (voxels) to the supported outline of the layer
    // below, for the voxels where it had to be worked out; +inf elsewhere.
    let mut outline = vec![f32::INFINITY; layer];
    let mut bridge_scratch = BridgeScratch::default();
    let mut stats = SupportStats::default();
    let (mut seen, mut piece, mut stack) = (vec![false; layer], Vec::new(), Vec::new());
    // Skin voxels of the layer below that were carved (see `flag::RIM`).
    let mut skin_carved = vec![false; layer];

    for k in vol.bed_layer..nz {
        let flags = &vol.flags[k * layer..(k + 1) * layer];
        let (lower, upper) = solid.split_at_mut(k * layer);
        let plane = &mut upper[..layer];
        let on_bed = k == vol.bed_layer;
        let below_ref = &below;
        let carved_ref = &skin_carved;
        let offsets_ref = &offsets;

        // Pass 1: overhang rule, row-parallel.
        let any = state
            .par_chunks_mut(nx)
            .zip(plane.par_chunks_mut(nx))
            .zip(cut.par_chunks_mut(nx))
            .zip(outline.par_chunks_mut(nx))
            .enumerate()
            .with_min_len(8)
            .map(|(j, (((st, row), cut_row), outline_row))| {
                let mut any = false;
                outline_row.fill(f32::INFINITY);
                for i in 0..nx {
                    let f = flags[j * nx + i];
                    if f & flag::INSIDE == 0 || row[i] <= 0.0 {
                        st[i] = VOID;
                        continue;
                    }
                    any = true;
                    // Skin is taken as printable, except along an opened face
                    // and wherever the skin under it had to be carved.
                    let held = f & (flag::SHELL | flag::RIM) == flag::SHELL && !carved_ref[j * nx + i];
                    if held || on_bed {
                        st[i] = SUPPORTED;
                        continue;
                    }
                    let depth = row[i] * inv_vox;
                    let mut best = below_ref[j * nx + i];
                    if reach + best >= depth {
                        // Standing on material at least as wide: nothing to clip.
                        st[i] = SUPPORTED;
                        continue;
                    }
                    for &(di, dj, dist) in offsets_ref {
                        if DEPTH_CAP - dist <= best {
                            break; // no farther neighbour can do better
                        }
                        let (ii, jj) = (i as i32 + di, j as i32 + dj);
                        if ii < 0 || jj < 0 || ii >= nx as i32 || jj >= ny as i32 {
                            continue;
                        }
                        best = best.max(below_ref[jj as usize * nx + ii as usize] - dist);
                    }
                    outline_row[i] = best;
                    let allowance = (reach + best) * vox;
                    if allowance > 0.02 * vox {
                        st[i] = SUPPORTED;
                        row[i] = row[i].min(allowance);
                    } else {
                        st[i] = UNSUPPORTED;
                        cut_row[i] = allowance.min(-0.02 * vox);
                    }
                }
                any
            })
            .reduce(|| false, |a, b| a | b);

        if !any {
            below.fill(NO_SUPPORT);
            skin_carved.fill(false);
            continue;
        }

        // Pass 2: bridge rule.
        if bridge_voxels >= 1.0 && state.contains(&UNSUPPORTED) {
            bridge::mark_bridges(
                &mut state,
                nx,
                ny,
                bridge_voxels,
                band_voxels,
                Mode::WholeRegions,
                &mut bridge_scratch,
            );
        }

        // Pass 2b: anchoring. Each voxel may rest on the very edge of the
        // layer below, so a small piece can end up standing on nothing more
        // than a sliver; once meshed and sliced, the sliver is a speck too
        // small to print and the piece, the tip of a strut, starts in
        // mid-air. Such tips are removed (the rest of the strut then follows
        // the overhang rule in the layers above).
        if !on_bed {
            let max_tip = (MAX_LOOSE_TIP / (vox * vox)).max(2.0) as usize;
            for start in 0..layer {
                if seen[start] || !matches!(state[start], SUPPORTED | BRIDGED) {
                    continue;
                }
                piece.clear();
                stack.push(start);
                seen[start] = true;
                let mut anchored = false;
                while let Some(p_idx) = stack.pop() {
                    piece.push(p_idx);
                    let (i, j) = ((p_idx % nx) as i32, (p_idx / nx) as i32);
                    anchored |= flags[p_idx] & (flag::SHELL | flag::RIM) == flag::SHELL && !skin_carved[p_idx];
                    for dj in -1..=1 {
                        for di in -1..=1 {
                            let (ii, jj) = (i + di, j + dj);
                            if ii < 0 || jj < 0 || ii >= nx as i32 || jj >= ny as i32 {
                                continue;
                            }
                            let q = jj as usize * nx + ii as usize;
                            anchored |= below[q] >= MIN_ANCHOR_DEPTH;
                            if !seen[q] && matches!(state[q], SUPPORTED | BRIDGED) {
                                seen[q] = true;
                                stack.push(q);
                            }
                        }
                    }
                }
                if !anchored && piece.len() <= max_tip {
                    for &p_idx in &piece {
                        state[p_idx] = UNSUPPORTED;
                        cut[p_idx] = -0.02 * vox;
                    }
                }
            }
            seen.fill(false);
        }

        // Pass 3: commit, and record how deep each kept voxel sits.
        if k > 0 {
            let under_flags = &vol.flags[(k - 1) * layer..k * layer];
            let under = &mut lower[(k - 1) * layer..];
            for p_idx in 0..layer {
                // Make the empty voxel under a clipped one agree with the cut
                // (bridges keep their flat underside).
                if outline[p_idx].is_finite()
                    && state[p_idx] != BRIDGED
                    && below[p_idx] == NO_SUPPORT
                    && under_flags[p_idx] & flag::INSIDE != 0
                    && under_flags[p_idx] & (flag::SHELL | flag::RIM) != flag::SHELL
                {
                    under[p_idx] = under[p_idx].min((outline[p_idx] * vox).max(-2.0 * vox));
                }
            }
        }
        for p_idx in 0..layer {
            skin_carved[p_idx] = state[p_idx] == UNSUPPORTED && flags[p_idx] & flag::SHELL != 0;
            below[p_idx] = match state[p_idx] {
                SUPPORTED => (plane[p_idx] * inv_vox).clamp(0.0, DEPTH_CAP),
                BRIDGED => {
                    stats.bridged_voxels += 1;
                    (plane[p_idx] * inv_vox).clamp(0.0, DEPTH_CAP)
                }
                UNSUPPORTED => {
                    plane[p_idx] = plane[p_idx].min(cut[p_idx]);
                    stats.removed_voxels += 1;
                    NO_SUPPORT
                }
                _ => NO_SUPPORT,
            };
        }
    }
    stats
}

/// Removes lattice that is not attached to the skin: it would be loose inside
/// the part. Returns the number of voxels removed.
///
/// Connectivity is by faces (6 neighbours), the conservative choice: pieces
/// that only touch along an edge or corner are treated as detached.
pub fn remove_loose(vol: &Volume, solid: &mut [f32]) -> usize {
    let grid = &vol.grid;
    let [nx, ny, nz] = grid.dims;
    let layer = nx * ny;
    let vox = grid.voxel() as f32;
    let n = grid.len();
    assert!(n <= u32::MAX as usize, "grid too large for the flood fill");

    let is_lattice = |idx: usize| vol.flags[idx] & (flag::INSIDE | flag::SHELL) == flag::INSIDE && solid[idx] > 0.0;
    let has_shell = vol.flags.par_iter().any(|&f| f & flag::SHELL != 0);

    let mut reached = vec![false; n];
    let mut stack: Vec<u32> = Vec::new();
    let neighbours = |idx: usize, out: &mut [usize; 6]| -> usize {
        let (i, j, k) = (idx % nx, (idx / nx) % ny, idx / layer);
        let mut c = 0;
        if i > 0 {
            out[c] = idx - 1;
            c += 1;
        }
        if i + 1 < nx {
            out[c] = idx + 1;
            c += 1;
        }
        if j > 0 {
            out[c] = idx - nx;
            c += 1;
        }
        if j + 1 < ny {
            out[c] = idx + nx;
            c += 1;
        }
        if k > 0 {
            out[c] = idx - layer;
            c += 1;
        }
        if k + 1 < nz {
            out[c] = idx + layer;
            c += 1;
        }
        c
    };

    // Seeds: lattice voxels touching the skin (or, for a model with no skin
    // at all, lattice resting on the build plate).
    let mut nb = [0usize; 6];
    for idx in 0..n {
        if !is_lattice(idx) {
            continue;
        }
        let seed = if has_shell {
            let c = neighbours(idx, &mut nb);
            nb[..c].iter().any(|&m| vol.flags[m] & flag::SHELL != 0)
        } else {
            idx / layer == vol.bed_layer
        };
        if seed {
            reached[idx] = true;
            stack.push(idx as u32);
        }
    }
    while let Some(idx) = stack.pop() {
        let c = neighbours(idx as usize, &mut nb);
        for &m in &nb[..c] {
            if !reached[m] && is_lattice(m) {
                reached[m] = true;
                stack.push(m as u32);
            }
        }
    }

    let mut removed = 0usize;
    for (idx, value) in solid.iter_mut().enumerate() {
        let lattice = vol.flags[idx] & (flag::INSIDE | flag::SHELL) == flag::INSIDE && *value > 0.0;
        if lattice && !reached[idx] {
            *value = -0.5 * vox;
            removed += 1;
        }
    }
    removed
}

/// Removes lattice features thinner than about `min_feature` millimetres (a
/// morphological opening). Returns the number of voxels removed.
///
/// The field is itself a distance to the surface, so "this voxel can host a
/// ball of radius r" is read from it directly, with sub-voxel accuracy; a
/// distance transform then tells which voxels those balls cover.
pub fn remove_thin(vol: &Volume, solid: &mut [f32], min_feature: f64) -> usize {
    let grid = &vol.grid;
    let vox = grid.voxel() as f32;
    let radius = 0.5 * min_feature as f32;
    if radius < 0.5 * vox {
        return 0; // below what the grid can resolve
    }
    // Half a voxel of tolerance: ridges are sampled, and lightly smoothed.
    let threshold = radius - 0.5 * vox;
    let reach = {
        let solid_ref: &[f32] = solid;
        crate::edt::distance_to(grid.dims, |i| vol.flags[i] & flag::INSIDE != 0 && solid_ref[i] >= threshold)
    };
    let cover = radius / vox + 1.0;
    solid
        .par_iter_mut()
        .zip(reach.par_iter())
        .zip(vol.flags.par_iter())
        .map(|((s, &r), &f)| {
            let lattice = f & (flag::INSIDE | flag::SHELL) == flag::INSIDE;
            if lattice && *s > 0.0 && r > cover {
                *s = (*s).min(((cover - r) * vox).min(-0.02 * vox));
                1usize
            } else {
                0
            }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Grid;

    /// A hand-made volume: a box of inside voxels with a one-voxel skin on the
    /// walls and floor, open at the top.
    fn test_volume(n: usize) -> Volume {
        let grid = Grid { origin: [0.0; 3], spacing: [1.0; 3], dims: [n, n, n] };
        let mut flags = vec![0u8; grid.len()];
        for k in 1..n - 1 {
            for j in 1..n - 1 {
                for i in 1..n - 1 {
                    let mut f = flag::INSIDE | flag::EXT;
                    if i == 1 || j == 1 || i == n - 2 || j == n - 2 || k == 1 {
                        f |= flag::SHELL;
                    }
                    flags[grid.index(i, j, k)] = f;
                }
            }
        }
        let len = grid.len();
        Volume {
            grid,
            flags,
            dist: vec![0.0; len],
            ceiling: vec![crate::volume::NO_CEILING; len],
            ceiling_flatness: vec![0; len],
            bed_layer: 1,
            inside_voxels: 0,
            core_voxels: 0,
        }
    }

    fn params(overhang: f64, bridge: f64) -> Params {
        Params { max_overhang_deg: overhang, max_bridge: bridge, ..Params::default() }
    }

    fn base_field(vol: &Volume) -> Vec<f32> {
        vol.flags.iter().map(|&f| if f & flag::SHELL != 0 { 1.0 } else { -1.0 }).collect()
    }

    #[test]
    fn vertical_column_survives_and_floating_blob_is_removed() {
        let vol = test_volume(16);
        let g = vol.grid.clone();
        let mut solid = base_field(&vol);
        for k in 2..12 {
            solid[g.index(7, 7, k)] = 1.0; // column standing on the floor
        }
        for k in 8..10 {
            solid[g.index(11, 11, k)] = 1.0; // blob in mid-air
        }
        let stats = make_printable(&vol, &mut solid, &params(45.0, 0.0));
        for k in 2..12 {
            assert!(solid[g.index(7, 7, k)] > 0.0);
        }
        for k in 8..10 {
            assert!(solid[g.index(11, 11, k)] < 0.0);
        }
        assert_eq!(stats.removed_voxels, 2);
    }

    #[test]
    fn forty_five_degree_stair_survives_but_steeper_does_not() {
        let vol = test_volume(24);
        let g = vol.grid.clone();
        // One voxel sideways per layer: exactly 45 degrees.
        let mut ok = base_field(&vol);
        for s in 0..10 {
            ok[g.index(4 + s, 10, 2 + s)] = 1.0;
            ok[g.index(5 + s, 10, 2 + s)] = 1.0;
        }
        make_printable(&vol, &mut ok, &params(45.0, 0.0));
        for s in 0..10 {
            assert!(ok[g.index(5 + s, 10, 2 + s)] > 0.0, "step {s} was cut");
        }
        // Three voxels sideways per layer: about 72 degrees.
        let mut steep = base_field(&vol);
        for s in 0..5 {
            for w in 0..3 {
                steep[g.index(4 + 3 * s + w, 10, 2 + s)] = 1.0;
            }
        }
        make_printable(&vol, &mut steep, &params(45.0, 0.0));
        assert!(steep[g.index(4 + 12 + 2, 10, 6)] < 0.0, "72 degree overhang survived");
        // A 75 degree limit accepts it.
        let mut relaxed = base_field(&vol);
        for s in 0..5 {
            for w in 0..3 {
                relaxed[g.index(4 + 3 * s + w, 10, 2 + s)] = 1.0;
            }
        }
        make_printable(&vol, &mut relaxed, &params(75.0, 0.0));
        assert!(relaxed[g.index(4 + 12 + 2, 10, 6)] > 0.0);
    }

    #[test]
    fn short_span_bridges_long_span_and_cantilever_do_not() {
        let vol = test_volume(32);
        let g = vol.grid.clone();
        // Pillars and beam are 3 voxels deep: a bridge is a band, not a hair.
        let build = |gap: usize, second_pillar: bool| {
            let mut solid = base_field(&vol);
            for j in 14..17 {
                for k in 2..10 {
                    solid[g.index(5, j, k)] = 1.0;
                    if second_pillar {
                        solid[g.index(6 + gap, j, k)] = 1.0;
                    }
                }
                for i in 5..=6 + gap {
                    solid[g.index(i, j, 10)] = 1.0; // beam on top
                }
            }
            solid
        };
        // 6-voxel gap, 8 mm allowed: a bridge.
        let mut a = build(6, true);
        let stats = make_printable(&vol, &mut a, &params(45.0, 8.0));
        assert!(a[g.index(9, 15, 10)] > 0.0);
        assert!(stats.bridged_voxels > 0);
        // Same beam, 3 mm allowed: too long, the middle goes.
        let mut b = build(6, true);
        make_printable(&vol, &mut b, &params(45.0, 3.0));
        assert!(b[g.index(9, 15, 10)] < 0.0);
        // One pillar only: a cantilever, never a bridge.
        let mut c = build(6, false);
        make_printable(&vol, &mut c, &params(45.0, 8.0));
        assert!(c[g.index(11, 15, 10)] < 0.0);
        assert!(c[g.index(6, 15, 10)] > 0.0, "first overhang voxel is within 45 degrees");
    }

    #[test]
    fn loose_pieces_are_removed_and_attached_ones_kept() {
        let vol = test_volume(16);
        let g = vol.grid.clone();
        let mut solid = base_field(&vol);
        for i in 2..8 {
            solid[g.index(i, 7, 6)] = 1.0; // touches the wall skin at i = 1
        }
        solid[g.index(10, 10, 10)] = 1.0; // isolated
        let removed = remove_loose(&vol, &mut solid);
        assert_eq!(removed, 1);
        assert!(solid[g.index(5, 7, 6)] > 0.0);
        assert!(solid[g.index(10, 10, 10)] < 0.0);
    }

    #[test]
    fn thin_plate_is_removed_thick_plate_kept() {
        let vol = test_volume(24);
        let g = vol.grid.clone();
        let mut solid = base_field(&vol);
        // Field values are distances to the surface, as in the real pipeline.
        for j in 2..22 {
            for k in 2..22 {
                solid[g.index(8, j, k)] = 0.5; // 1 voxel thick
                for (n, i) in (14..18).enumerate() {
                    solid[g.index(i, j, k)] = [0.5, 1.5, 1.5, 0.5][n]; // 4 voxels thick
                }
            }
        }
        let removed = remove_thin(&vol, &mut solid, 3.0);
        assert!(removed > 0);
        assert!(solid[g.index(8, 12, 12)] < 0.0);
        assert!(solid[g.index(15, 12, 12)] > 0.0);
        assert!(solid[g.index(14, 12, 12)] > 0.0);
    }
}
