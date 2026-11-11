//! Bridge detection within one layer, shared by the generator and the
//! printability check.
//!
//! A bridge is a band of parallel strands, each resting on supported material
//! at both ends. An unsupported pixel is accepted as bridged when
//!
//! 1. it lies on a straight run of unsupported pixels, along one of four
//!    directions, that touches supported material at both ends and is no
//!    longer than the limit; and
//! 2. that strand is not on its own: across the strand direction, strands
//!    accepted for the same direction (or supported material) continue for at
//!    least `min_width`.
//!
//! The second condition is what separates a real bridge from the stray
//! one-pixel chords that can always be drawn across the corner of a ledge: a
//! nozzle cannot lay those down, and what they would hold is a cantilever.
//! Different parts of a region may use different directions, so a winding
//! slot bridges as long as it is narrow everywhere; where it is locally too
//! wide (typically where slots meet) only that part is rejected.
//!
//! The generator asks for more ([`Mode::WholeRegions`]): a connected
//! unsupported region is kept only if all of it bridges. A half-bridged
//! region would leave a ragged edge of strands next to a carved hole; carving
//! the whole region instead gives a clean pointed arch.

/// Pixel states. Anything other than `VOID`, `UNSUPPORTED` and `BRIDGED` is
/// treated as supported material a bridge may rest on.
pub const VOID: u8 = 0;
pub const SUPPORTED: u8 = 1;
pub const UNSUPPORTED: u8 = 2;
pub const BRIDGED: u8 = 3;

/// `(di, dj, step length in pixels)` of the four scan directions; direction
/// `d ^ 1` is perpendicular to direction `d`.
const DIRS: [(i32, i32, f32); 4] =
    [(1, 0, 1.0), (0, 1, 1.0), (1, 1, std::f32::consts::SQRT_2), (1, -1, std::f32::consts::SQRT_2)];

/// How partially bridgeable regions are treated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Accept every pixel that bridges (used to assess a finished part).
    PerPixel,
    /// Accept a connected region only if every pixel of it bridges (used to
    /// generate clean geometry).
    WholeRegions,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BridgeStats {
    /// Pixels promoted to `BRIDGED`.
    pub bridged: usize,
    /// Longest strand among accepted bridges (pixels).
    pub longest: f32,
    /// Widest gap that is enclosed by supports but too long to bridge: the
    /// shortest strand that would have crossed its worst pixel (pixels).
    pub longest_rejected: f32,
}

/// Reusable buffers for [`mark_bridges`].
#[derive(Default)]
pub struct BridgeScratch {
    /// The unsupported pixels of the layer.
    todo: Vec<usize>,
    /// Position of each unsupported pixel in `todo` (stale elsewhere).
    slot: Vec<u32>,
    /// Two-ended run length through each `todo` pixel, per direction
    /// (NaN = not worked out yet, infinite = open at an end).
    span: Vec<[f32; 4]>,
    best: Vec<f32>,
    visited: Vec<bool>,
    stack: Vec<usize>,
    region: Vec<usize>,
}

#[inline]
fn is_anchor(s: u8) -> bool {
    s != VOID && s != UNSUPPORTED && s != BRIDGED
}

/// Length (pixels) of the run of unsupported pixels through `p` along
/// direction `d`, if it touches supported material at both ends; infinite
/// otherwise. Runs longer than `cap` are not followed to their end and count
/// as open.
fn run_through(state: &[u8], nx: usize, ny: usize, p: usize, d: usize, cap: f32) -> f32 {
    let (di, dj, step) = DIRS[d];
    let (i0, j0) = ((p % nx) as i32, (p / nx) as i32);
    let mut count = 1usize;
    for sign in [1i32, -1] {
        let (mut i, mut j) = (i0 + sign * di, j0 + sign * dj);
        loop {
            if i < 0 || j < 0 || i >= nx as i32 || j >= ny as i32 {
                return f32::INFINITY;
            }
            let s = state[j as usize * nx + i as usize];
            if s == UNSUPPORTED {
                count += 1;
                if count as f32 * step > cap {
                    return f32::INFINITY;
                }
                i += sign * di;
                j += sign * dj;
            } else if is_anchor(s) {
                break;
            } else {
                return f32::INFINITY;
            }
        }
    }
    count as f32 * step
}

/// Promotes bridgeable `UNSUPPORTED` pixels to `BRIDGED`. `max_len` is the
/// bridge limit and `min_width` the narrowest printable band, in pixels.
///
/// Work is proportional to the number of unsupported pixels, not to the size
/// of the layer. Gaps wider than four times the limit are not measured.
pub fn mark_bridges(
    state: &mut [u8],
    nx: usize,
    ny: usize,
    max_len: f32,
    min_width: f32,
    mode: Mode,
    scratch: &mut BridgeScratch,
) -> BridgeStats {
    let n = nx * ny;
    let mut stats = BridgeStats::default();
    let BridgeScratch { todo, slot, span, best, visited, stack, region } = scratch;
    todo.clear();
    todo.extend((0..n).filter(|&p| state[p] == UNSUPPORTED));
    if todo.is_empty() {
        return stats;
    }
    slot.resize(n, 0);
    for (k, &p) in todo.iter().enumerate() {
        slot[p] = k as u32;
    }
    span.clear();
    span.resize(todo.len(), [f32::NAN; 4]);
    best.clear();
    best.resize(todo.len(), f32::INFINITY);
    let cap = 4.0 * max_len.max(1.0);

    // Memoised run length through an unsupported pixel.
    let run_of = |span: &mut Vec<[f32; 4]>, state: &[u8], p: usize, d: usize| -> f32 {
        let k = slot[p] as usize;
        if span[k][d].is_nan() {
            span[k][d] = run_through(state, nx, ny, p, d, cap);
        }
        span[k][d]
    };

    for k in 0..todo.len() {
        let p = todo[k];
        let (i0, j0) = ((p % nx) as i32, (p / nx) as i32);
        for d in 0..4 {
            // 1. A strand through p, resting on supports at both ends.
            let length = run_of(span, state, p, d);
            if length > max_len {
                continue;
            }
            // 2. ...that is part of a band: across it, equally short strands
            //    (or supported material) continue for `min_width`.
            let (di, dj, step) = DIRS[d ^ 1];
            let mut width = 1usize;
            'sides: for sign in [1i32, -1] {
                let (mut i, mut j) = (i0 + sign * di, j0 + sign * dj);
                while width as f32 * step < min_width {
                    if i < 0 || j < 0 || i >= nx as i32 || j >= ny as i32 {
                        continue 'sides;
                    }
                    let q = j as usize * nx + i as usize;
                    let s = state[q];
                    let continues = is_anchor(s) || (s == UNSUPPORTED && run_of(span, state, q, d) <= max_len);
                    if !continues {
                        continue 'sides;
                    }
                    width += 1;
                    i += sign * di;
                    j += sign * dj;
                }
                break;
            }
            if width as f32 * step >= min_width {
                best[k] = best[k].min(length);
            }
        }
    }

    // 3. Optionally, drop regions that do not bridge completely.
    if mode == Mode::WholeRegions {
        visited.clear();
        visited.resize(todo.len(), false);
        for start in 0..todo.len() {
            if visited[start] {
                continue;
            }
            region.clear();
            stack.push(start);
            visited[start] = true;
            let mut complete = true;
            while let Some(k) = stack.pop() {
                region.push(k);
                complete &= best[k].is_finite();
                let p = todo[k];
                let (i, j) = ((p % nx) as i32, (p / nx) as i32);
                for dj in -1..=1 {
                    for di in -1..=1 {
                        let (ii, jj) = (i + di, j + dj);
                        if ii < 0 || jj < 0 || ii >= nx as i32 || jj >= ny as i32 {
                            continue;
                        }
                        let q = jj as usize * nx + ii as usize;
                        if state[q] == UNSUPPORTED && !visited[slot[q] as usize] {
                            visited[slot[q] as usize] = true;
                            stack.push(slot[q] as usize);
                        }
                    }
                }
            }
            if !complete {
                for &k in region.iter() {
                    best[k] = f32::INFINITY;
                }
            }
        }
    }

    for k in 0..todo.len() {
        if best[k].is_finite() {
            state[todo[k]] = BRIDGED;
            stats.bridged += 1;
            stats.longest = stats.longest.max(best[k]);
        } else {
            // Not bridged. If it is enclosed by supports, how long a strand
            // would it have taken?
            let p = todo[k];
            let shortest = (0..4).map(|d| run_of(span, state, p, d)).fold(f32::INFINITY, f32::min);
            if shortest.is_finite() && shortest > max_len {
                stats.longest_rejected = stats.longest_rejected.max(shortest);
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(rows: &[&str]) -> (Vec<u8>, usize, usize) {
        let nx = rows[0].len();
        let state = rows
            .iter()
            .flat_map(|r| {
                r.bytes().map(|b| match b {
                    b'#' => SUPPORTED,
                    b'u' => UNSUPPORTED,
                    _ => VOID,
                })
            })
            .collect();
        (state, nx, rows.len())
    }

    fn count(state: &[u8], what: u8) -> usize {
        state.iter().filter(|&&s| s == what).count()
    }

    fn run(rows: &[&str], max_len: f32, min_width: f32) -> (Vec<u8>, BridgeStats) {
        run_mode(rows, max_len, min_width, Mode::PerPixel)
    }

    fn run_mode(rows: &[&str], max_len: f32, min_width: f32, mode: Mode) -> (Vec<u8>, BridgeStats) {
        let (mut s, nx, ny) = grid(rows);
        let stats = mark_bridges(&mut s, nx, ny, max_len, min_width, mode, &mut BridgeScratch::default());
        (s, stats)
    }

    #[test]
    fn span_between_two_supports_is_a_bridge() {
        let (s, stats) = run(&["#uuuu#", "#uuuu#", "#uuuu#"], 5.0, 3.0);
        assert_eq!(stats.bridged, 12);
        assert_eq!(stats.longest, 4.0);
        assert_eq!(count(&s, UNSUPPORTED), 0);
    }

    #[test]
    fn too_long_or_one_sided_is_not() {
        let (_, stats) = run(&["#uuuu#", "#uuuu#", "#uuuu#"], 3.0, 3.0);
        assert_eq!(stats.bridged, 0);
        assert_eq!(stats.longest_rejected, 4.0);

        let (_, stats) = run(&["#uuuu.", "#uuuu.", "#uuuu."], 9.0, 3.0);
        assert_eq!(stats.bridged, 0);
        assert_eq!(stats.longest_rejected, 0.0);
    }

    #[test]
    fn lone_strand_is_not_a_bridge_but_one_beside_material_is() {
        // One pixel wide, void on both sides.
        let (_, stats) = run(&["......", "#uuuu#", "......"], 5.0, 3.0);
        assert_eq!(stats.bridged, 0);
        // The same strand lying against supported material.
        let (_, stats) = run(&["######", "######", "#uuuu#", "......"], 5.0, 3.0);
        assert_eq!(stats.bridged, 4);
    }

    #[test]
    fn stray_chords_across_the_corner_of_a_ledge_are_rejected() {
        // A ledge sticking out of a wall, with a small post near one corner.
        // Chords can be drawn from the wall to the post, but the ledge is a
        // cantilever and those chords would be isolated strands.
        let (s, stats) = run(&["#uuuuuuuu", "#uuuuuuuu", "#uuuuuuuu", "#uuuuuuuu", "#uuuuuuuu", "....#...."], 8.0, 3.0);
        assert_eq!(stats.bridged, 0, "{s:?}");
    }

    #[test]
    fn winding_slot_bridges_and_only_the_wide_part_fails() {
        let rows = [
            "############",
            "#uuuuuuuuu##",
            "#uuuuuuuuu##",
            "########uu##",
            "########uu##",
            "##uuuuuuuu##",
            "##uuuuuuuu##",
            "############",
        ];
        let (s, stats) = run(&rows, 2.0, 2.0);
        // Everything bridges except the inside of the two bends, where the
        // slot is wider than 2 pixels measured in any direction.
        assert_eq!(count(&s, BRIDGED), 32, "{s:?}");
        assert_eq!(count(&s, UNSUPPORTED), 6, "{s:?}");
        assert!(stats.longest <= 2.0);
        // A slightly longer limit covers the bends as well.
        let (s, _) = run(&rows, 3.0, 2.0);
        assert_eq!(count(&s, UNSUPPORTED), 0, "{s:?}");

        // Same slot with a 5 x 5 chamber in it: the chamber's middle cannot
        // be reached by a 2-pixel strand, the rest still bridges.
        let rows = [
            "############",
            "#uuuuuuuuu##",
            "#uuuuuuuuu##",
            "#####uuuuu##",
            "#####uuuuu##",
            "#####uuuuu##",
            "############",
        ];
        let (s, stats) = run(&rows, 2.0, 2.0);
        assert!(count(&s, BRIDGED) >= 8);
        assert!(count(&s, UNSUPPORTED) >= 9);
        assert!(stats.longest_rejected >= 5.0);
        // Asked for whole regions, nothing of it is accepted...
        let (s, stats) = run_mode(&rows, 2.0, 2.0, Mode::WholeRegions);
        assert_eq!(stats.bridged, 0);
        assert_eq!(count(&s, BRIDGED), 0);
        // ...while a region that bridges completely still is.
        let (_, stats) = run_mode(&["#uuuu#", "#uuuu#", "#uuuu#"], 5.0, 3.0, Mode::WholeRegions);
        assert_eq!(stats.bridged, 12);
    }

    #[test]
    fn diagonal_slot_bridges() {
        let rows =
            ["#########", "#uu######", "##uu#####", "###uu####", "####uu###", "#####uu##", "######uu#", "#########"];
        let (s, _) = run(&rows, 3.0, 2.0);
        assert_eq!(count(&s, UNSUPPORTED), 0, "{s:?}");
    }
}
