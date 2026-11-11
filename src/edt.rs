//! Exact Euclidean distance transform (Felzenszwalb & Huttenlocher), separable
//! and multi-threaded.
//!
//! Distances are measured between voxel centres, in voxel units. Nothing is
//! assumed outside the grid (no implicit background border).

use crate::util::SharedSlice;
use rayon::prelude::*;

/// Stand-in for "no site reachable along this line".
pub const FAR: f32 = 1.0e20;

/// Scratch buffers for the 1-D lower-envelope pass.
pub struct Scratch {
    f: Vec<f32>,
    d: Vec<f32>,
    v: Vec<u32>,
    z: Vec<f32>,
}

impl Scratch {
    pub fn new(n: usize) -> Self {
        Self { f: vec![0.0; n], d: vec![0.0; n], v: vec![0; n], z: vec![0.0; n + 1] }
    }
}

/// 1-D squared distance transform of the sampled function `f` (values `>= FAR`
/// mean "no parabola here"). Writes `min_p (q - p)^2 + f[p]` into `d`.
fn envelope(f: &[f32], d: &mut [f32], v: &mut [u32], z: &mut [f32]) {
    let n = f.len();
    let mut k: usize = 0;
    let mut any = false;
    for q in 0..n {
        let fq = f[q];
        if fq >= FAR {
            continue;
        }
        if !any {
            any = true;
            v[0] = q as u32;
            z[0] = f32::NEG_INFINITY;
            z[1] = f32::INFINITY;
            continue;
        }
        let qf = q as f64;
        let gq = fq as f64 + qf * qf;
        loop {
            let p = v[k] as usize;
            let pf = p as f64;
            let s = ((gq - (f[p] as f64 + pf * pf)) / (2.0 * (qf - pf))) as f32;
            if s <= z[k] {
                // z[0] is -inf, so k never underflows.
                k -= 1;
            } else {
                k += 1;
                v[k] = q as u32;
                z[k] = s;
                z[k + 1] = f32::INFINITY;
                break;
            }
        }
    }
    if !any {
        d.fill(FAR);
        return;
    }
    let mut j = 0usize;
    for (q, out) in d.iter_mut().enumerate() {
        let qf = q as f32;
        while z[j + 1] < qf {
            j += 1;
        }
        let p = v[j] as usize;
        let delta = qf - p as f32;
        *out = delta * delta + f[p];
    }
}

/// First pass: squared distance to the nearest site along one row.
fn row_pass(row: &mut [f32], is_site: impl Fn(usize) -> bool) {
    let n = row.len();
    let mut dist = FAR;
    for (i, cell) in row.iter_mut().enumerate() {
        dist = if is_site(i) { 0.0 } else { dist + 1.0 };
        *cell = dist;
    }
    dist = FAR;
    for i in (0..n).rev() {
        dist = if row[i] == 0.0 { 0.0 } else { dist + 1.0 };
        let best = row[i].min(dist);
        row[i] = if best >= 1.0e9 { FAR } else { best * best };
    }
}

/// Like [`envelope`], but also reports which sample each minimum came from.
fn envelope_arg(f: &[f32], d: &mut [f32], arg: &mut [u32], v: &mut [u32], z: &mut [f32]) {
    let n = f.len();
    let mut k: usize = 0;
    let mut any = false;
    for q in 0..n {
        let fq = f[q];
        if fq >= FAR {
            continue;
        }
        if !any {
            any = true;
            v[0] = q as u32;
            z[0] = f32::NEG_INFINITY;
            z[1] = f32::INFINITY;
            continue;
        }
        let qf = q as f64;
        let gq = fq as f64 + qf * qf;
        loop {
            let p = v[k] as usize;
            let pf = p as f64;
            let s = ((gq - (f[p] as f64 + pf * pf)) / (2.0 * (qf - pf))) as f32;
            if s <= z[k] {
                k -= 1;
            } else {
                k += 1;
                v[k] = q as u32;
                z[k] = s;
                z[k + 1] = f32::INFINITY;
                break;
            }
        }
    }
    if !any {
        d.fill(FAR);
        arg.fill(u32::MAX);
        return;
    }
    let mut j = 0usize;
    for q in 0..n {
        let qf = q as f32;
        while z[j + 1] < qf {
            j += 1;
        }
        let p = v[j] as usize;
        let delta = qf - p as f32;
        d[q] = delta * delta + f[p];
        arg[q] = p as u32;
    }
}

/// Buffers for [`feature_transform_2d`].
pub struct FeatureScratch {
    line: Scratch,
    arg: Vec<u32>,
    row_site: Vec<u32>,
}

impl FeatureScratch {
    pub fn new(nx: usize, ny: usize) -> Self {
        Self { line: Scratch::new(nx.max(ny)), arg: vec![0; ny], row_site: vec![0; nx * ny] }
    }
}

/// 2-D feature transform of one `nx * ny` layer: for every pixel, the
/// distance (pixels) to the nearest site and that site's index
/// (`u32::MAX` if the layer has no sites).
pub fn feature_transform_2d<F>(
    nx: usize,
    ny: usize,
    dist: &mut [f32],
    site: &mut [u32],
    is_site: F,
    scratch: &mut FeatureScratch,
) where
    F: Fn(usize) -> bool,
{
    debug_assert!(dist.len() == nx * ny && site.len() == nx * ny);
    let FeatureScratch { line, arg, row_site } = scratch;
    // Rows: nearest site along x, with its column.
    for j in 0..ny {
        let row = &mut dist[j * nx..(j + 1) * nx];
        let sites = &mut row_site[j * nx..(j + 1) * nx];
        let mut last: Option<usize> = None;
        for i in 0..nx {
            if is_site(j * nx + i) {
                last = Some(i);
            }
            match last {
                Some(s) => {
                    row[i] = (i - s) as f32;
                    sites[i] = s as u32;
                }
                None => {
                    row[i] = FAR;
                    sites[i] = u32::MAX;
                }
            }
        }
        last = None;
        for i in (0..nx).rev() {
            if sites[i] == i as u32 {
                last = Some(i);
            }
            if let Some(s) = last {
                let d = (s - i) as f32;
                if d < row[i] {
                    row[i] = d;
                    sites[i] = s as u32;
                }
            }
            row[i] = if row[i] >= 1.0e9 { FAR } else { row[i] * row[i] };
        }
    }
    // Columns: lower envelope, remembering which row won.
    let Scratch { f, d, v, z } = line;
    let (f, d, v, z) = (&mut f[..ny], &mut d[..ny], &mut v[..ny], &mut z[..ny + 1]);
    for i in 0..nx {
        for j in 0..ny {
            f[j] = dist[j * nx + i];
        }
        envelope_arg(f, d, arg, v, z);
        for j in 0..ny {
            dist[j * nx + i] = d[j].sqrt();
            site[j * nx + i] = match arg[j] {
                u32::MAX => u32::MAX,
                row => match row_site[row as usize * nx + i] {
                    u32::MAX => u32::MAX,
                    col => row * nx as u32 + col,
                },
            };
        }
    }
}

/// 2-D version for a single `nx * ny` layer, single-threaded: distance (in
/// pixels) to the nearest pixel for which `is_site(index)` is true, written to
/// `out`. `scratch` must hold at least `max(nx, ny)` entries.
pub fn distance_to_2d<F>(nx: usize, ny: usize, out: &mut [f32], is_site: F, scratch: &mut Scratch)
where
    F: Fn(usize) -> bool,
{
    debug_assert_eq!(out.len(), nx * ny);
    for (j, row) in out.chunks_mut(nx).enumerate() {
        row_pass(row, |i| is_site(j * nx + i));
    }
    let Scratch { f, d, v, z } = scratch;
    let (f, d, v, z) = (&mut f[..ny], &mut d[..ny], &mut v[..ny], &mut z[..ny + 1]);
    for i in 0..nx {
        for j in 0..ny {
            f[j] = out[j * nx + i];
        }
        envelope(f, d, v, z);
        for j in 0..ny {
            out[j * nx + i] = d[j];
        }
    }
    for value in out.iter_mut() {
        *value = value.sqrt();
    }
}

/// Distance (in voxels) from every voxel to the nearest voxel for which
/// `is_site(index)` is true. Site voxels get 0. If there are no sites at all
/// every voxel gets a very large value.
pub fn distance_to<F>(dims: [usize; 3], is_site: F) -> Vec<f32>
where
    F: Fn(usize) -> bool + Sync,
{
    let [nx, ny, nz] = dims;
    let layer = nx * ny;
    let mut out = vec![0f32; layer * nz];
    if out.is_empty() {
        return out;
    }

    // X: contiguous rows.
    out.par_chunks_mut(nx).enumerate().for_each(|(row, line)| {
        let base = row * nx;
        row_pass(line, |i| is_site(base + i));
    });

    // Y: columns inside each contiguous Z layer.
    out.par_chunks_mut(layer).for_each_init(
        || Scratch::new(ny),
        |s, plane| {
            for i in 0..nx {
                for j in 0..ny {
                    s.f[j] = plane[j * nx + i];
                }
                envelope(&s.f, &mut s.d, &mut s.v, &mut s.z);
                for j in 0..ny {
                    plane[j * nx + i] = s.d[j];
                }
            }
        },
    );

    // Z: for each j, the XZ slab is made of nz disjoint rows. Copy it to a
    // compact buffer (sequential row copies are cache friendly), transform
    // its columns, copy back.
    if nz > 1 {
        let shared = SharedSlice::new(&mut out);
        (0..ny).into_par_iter().for_each_init(
            || (vec![0f32; nx * nz], Scratch::new(nz)),
            |(slab, s), j| {
                for k in 0..nz {
                    // SAFETY: rows of different `j` never overlap, and each
                    // `j` is handled by exactly one task.
                    let row = unsafe { shared.slice((k * ny + j) * nx, nx) };
                    slab[k * nx..(k + 1) * nx].copy_from_slice(row);
                }
                for i in 0..nx {
                    for k in 0..nz {
                        s.f[k] = slab[k * nx + i];
                    }
                    envelope(&s.f, &mut s.d, &mut s.v, &mut s.z);
                    for k in 0..nz {
                        slab[k * nx + i] = s.d[k];
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

    out.par_iter_mut().for_each(|v| *v = v.sqrt());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(dims: [usize; 3], sites: &[bool]) -> Vec<f32> {
        let [nx, ny, nz] = dims;
        let pts: Vec<[i64; 3]> = (0..sites.len())
            .filter(|&i| sites[i])
            .map(|i| [(i % nx) as i64, ((i / nx) % ny) as i64, (i / (nx * ny)) as i64])
            .collect();
        let mut out = vec![0f32; nx * ny * nz];
        for (idx, o) in out.iter_mut().enumerate() {
            let p = [(idx % nx) as i64, ((idx / nx) % ny) as i64, (idx / (nx * ny)) as i64];
            let best = pts.iter().map(|s| (0..3).map(|a| (s[a] - p[a]).pow(2)).sum::<i64>()).min().unwrap();
            *o = (best as f32).sqrt();
        }
        out
    }

    #[test]
    fn matches_brute_force_on_random_sites() {
        let dims = [13, 9, 11];
        let mut rng = crate::util::Rng::new(42);
        let n = dims[0] * dims[1] * dims[2];
        for density in [0.01, 0.1, 0.6] {
            let mut sites: Vec<bool> = (0..n).map(|_| rng.uniform() < density).collect();
            sites[n / 2] = true;
            let fast = distance_to(dims, |i| sites[i]);
            let slow = brute(dims, &sites);
            for i in 0..n {
                assert!((fast[i] - slow[i]).abs() < 1e-4, "idx {i}: {} vs {}", fast[i], slow[i]);
            }
        }
    }

    #[test]
    fn feature_transform_returns_the_nearest_site() {
        let (nx, ny) = (17, 11);
        let mut rng = crate::util::Rng::new(5);
        let sites: Vec<bool> = (0..nx * ny).map(|_| rng.uniform() < 0.06).collect();
        let mut dist = vec![0f32; nx * ny];
        let mut site = vec![0u32; nx * ny];
        feature_transform_2d(nx, ny, &mut dist, &mut site, |i| sites[i], &mut FeatureScratch::new(nx, ny));
        for p in 0..nx * ny {
            let (x, y) = ((p % nx) as f32, (p / nx) as f32);
            let best = (0..nx * ny)
                .filter(|&q| sites[q])
                .map(|q| ((q % nx) as f32 - x).hypot((q / nx) as f32 - y))
                .fold(f32::INFINITY, f32::min);
            assert!((dist[p] - best).abs() < 1e-4);
            let s = site[p] as usize;
            assert!(sites[s]);
            let to_site = ((s % nx) as f32 - x).hypot((s / nx) as f32 - y);
            assert!((to_site - best).abs() < 1e-4, "site of {p} is not a nearest one");
        }
    }

    #[test]
    fn no_sites_gives_far_everywhere() {
        let d = distance_to([4, 4, 4], |_| false);
        assert!(d.iter().all(|&v| v > 1.0e9));
    }

    #[test]
    fn single_layer_grid() {
        let dims = [5, 5, 1];
        let d = distance_to(dims, |i| i == 12);
        assert_eq!(d[12], 0.0);
        assert!((d[0] - 8f32.sqrt()).abs() < 1e-5);
    }
}
