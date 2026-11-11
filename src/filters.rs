//! Small separable filters on voxel fields.

use crate::util::SharedSlice;
use rayon::prelude::*;

/// One pass of the binomial `[1 2 1] / 4` kernel along a strided line,
/// in place, with replicated ends.
#[inline]
fn binomial_line(line: &mut [f32]) {
    let n = line.len();
    if n < 2 {
        return;
    }
    let mut prev = line[0];
    for i in 0..n - 1 {
        let cur = line[i];
        line[i] = 0.25 * prev + 0.5 * cur + 0.25 * line[i + 1];
        prev = cur;
    }
    let cur = line[n - 1];
    line[n - 1] = 0.25 * prev + 0.75 * cur;
}

/// Light 3-D smoothing (binomial kernel on each axis, sigma about 0.7 voxel).
/// Removes the voxel-scale terracing that binary distance fields leave on the
/// inner side of the skin and rounds the creases of the lattice field.
pub fn smooth3(data: &mut [f32], dims: [usize; 3]) {
    let [nx, ny, nz] = dims;
    let layer = nx * ny;
    data.par_chunks_mut(layer).for_each_init(
        || vec![0f32; ny],
        |col, plane| {
            for row in plane.chunks_mut(nx) {
                binomial_line(row);
            }
            for i in 0..nx {
                for j in 0..ny {
                    col[j] = plane[j * nx + i];
                }
                binomial_line(col);
                for j in 0..ny {
                    plane[j * nx + i] = col[j];
                }
            }
        },
    );
    if nz < 2 {
        return;
    }
    let shared = SharedSlice::new(data);
    (0..ny).into_par_iter().for_each_init(
        || (vec![0f32; nx * nz], vec![0f32; nz]),
        |(slab, col), j| {
            for k in 0..nz {
                // SAFETY: each `j` owns rows `(k * ny + j) * nx ..` for all k;
                // different tasks use different `j`.
                let row = unsafe { shared.slice((k * ny + j) * nx, nx) };
                slab[k * nx..(k + 1) * nx].copy_from_slice(row);
            }
            for i in 0..nx {
                for k in 0..nz {
                    col[k] = slab[k * nx + i];
                }
                binomial_line(col);
                for k in 0..nz {
                    slab[k * nx + i] = col[k];
                }
            }
            for k in 0..nz {
                // SAFETY: as above.
                let row = unsafe { shared.slice_mut((k * ny + j) * nx, nx) };
                row.copy_from_slice(&slab[k * nx..(k + 1) * nx]);
            }
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_field_is_unchanged() {
        let dims = [5, 4, 3];
        let mut d = vec![2.5f32; 60];
        smooth3(&mut d, dims);
        assert!(d.iter().all(|&v| (v - 2.5).abs() < 1e-6));
    }

    #[test]
    fn impulse_spreads_and_keeps_its_sum() {
        let dims = [7, 7, 7];
        let mut d = vec![0f32; 343];
        d[3 + 7 * (3 + 7 * 3)] = 64.0;
        smooth3(&mut d, dims);
        let sum: f32 = d.iter().sum();
        assert!((sum - 64.0).abs() < 1e-3);
        assert!((d[3 + 7 * (3 + 7 * 3)] - 8.0).abs() < 1e-4);
        assert!((d[2 + 7 * (3 + 7 * 3)] - 4.0).abs() < 1e-4);
    }

    #[test]
    fn linear_ramp_is_preserved_inside() {
        let dims = [9, 3, 3];
        let mut d: Vec<f32> = (0..81).map(|i| (i % 9) as f32).collect();
        smooth3(&mut d, dims);
        for i in 1..8 {
            assert!((d[i] - i as f32).abs() < 1e-5);
        }
    }
}
