//! Band-limited Gaussian random fields as sums of plane waves.
//!
//! `g(p) = sqrt(2/N) * sum_i cos(k_i . p + phi_i)` has zero mean and unit
//! variance by construction, and can be evaluated anywhere without a global
//! FFT, so each grid row is computed independently and in parallel.
//!
//! Anisotropy is introduced by scaling wave vectors. To make structures
//! `e` times **longer** along a direction the wave-vector component along it
//! is **divided** by `e` (longer wavelength). Level-set surfaces then turn
//! towards that direction: elongating along Z yields mostly vertical sheets.

use crate::grid::Grid;
use crate::util::Rng;

#[derive(Clone, Debug)]
pub struct WaveNoise {
    /// Angular wave vectors (rad/mm).
    pub k: Vec<[f32; 3]>,
    pub phase: Vec<f32>,
    /// Amplitude of each wave, `sqrt(2 / N)`.
    pub amp: f32,
}

impl WaveNoise {
    /// * `wavelength` – dominant feature spacing (mm) before anisotropy.
    /// * `bandwidth` – relative spread of wave numbers.
    /// * `z_elongation` – stretch factor along Z (>= 1 elongates).
    /// * `stretch` – optional `(direction, factor)` elongation.
    pub fn new(
        rng: &mut Rng,
        wavelength: f64,
        waves: usize,
        bandwidth: f64,
        z_elongation: f64,
        stretch: Option<([f64; 3], f64)>,
    ) -> Self {
        let stretch = stretch.map(|(d, f)| {
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            ([d[0] / len, d[1] / len, d[2] / len], f)
        });
        let mut k = Vec::with_capacity(waves);
        let mut phase = Vec::with_capacity(waves);
        for _ in 0..waves {
            let mut d = [rng.normal(), rng.normal(), rng.normal()];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-12);
            let mag = (1.0 + bandwidth * rng.normal()).max(0.25) / wavelength;
            for c in &mut d {
                *c *= mag / len;
            }
            if let Some((u, factor)) = stretch {
                let par = d[0] * u[0] + d[1] * u[1] + d[2] * u[2];
                let delta = par * (1.0 / factor - 1.0);
                for a in 0..3 {
                    d[a] += delta * u[a];
                }
            }
            d[2] /= z_elongation;
            let tau = std::f64::consts::TAU;
            k.push([(d[0] * tau) as f32, (d[1] * tau) as f32, (d[2] * tau) as f32]);
            phase.push((rng.uniform() * tau) as f32);
        }
        Self { k, phase, amp: (2.0 / waves as f64).sqrt() as f32 }
    }

    pub fn len(&self) -> usize {
        self.k.len()
    }

    pub fn is_empty(&self) -> bool {
        self.k.is_empty()
    }

    /// Root-mean-square gradient magnitude (1/mm): `sqrt(mean |k|^2)`.
    pub fn grad_rms(&self) -> f64 {
        let sum: f64 =
            self.k.iter().map(|k| (k[0] as f64).powi(2) + (k[1] as f64).powi(2) + (k[2] as f64).powi(2)).sum();
        (sum / self.k.len() as f64).sqrt()
    }

    /// RMS of each gradient component (1/mm).
    pub fn grad_rms_axes(&self) -> [f64; 3] {
        let mut s = [0.0f64; 3];
        for k in &self.k {
            for a in 0..3 {
                s[a] += (k[a] as f64).powi(2);
            }
        }
        s.map(|v| (v / self.k.len() as f64).sqrt())
    }

    /// Field value and gradient at one point.
    pub fn eval(&self, p: [f64; 3]) -> (f32, [f32; 3]) {
        let mut v = 0.0f64;
        let mut g = [0.0f64; 3];
        for (k, &ph) in self.k.iter().zip(&self.phase) {
            let theta = k[0] as f64 * p[0] + k[1] as f64 * p[1] + k[2] as f64 * p[2] + ph as f64;
            let (s, c) = theta.sin_cos();
            v += c;
            for a in 0..3 {
                g[a] -= k[a] as f64 * s;
            }
        }
        let amp = self.amp as f64;
        ((v * amp) as f32, [(g[0] * amp) as f32, (g[1] * amp) as f32, (g[2] * amp) as f32])
    }
}

/// Per-axis `cos`/`sin` tables of every wave on a grid, so that a whole row is
/// evaluated with two multiply-adds per wave and voxel.
pub struct WaveTables {
    waves: usize,
    nx: usize,
    ny: usize,
    nz: usize,
    xr: Vec<f32>,
    xi: Vec<f32>,
    yr: Vec<f32>,
    yi: Vec<f32>,
    /// Z tables include the wave's phase.
    zr: Vec<f32>,
    zi: Vec<f32>,
    k: Vec<[f32; 3]>,
    amp: f32,
}

/// Per-row accumulators for value and gradient.
pub struct RowBuffers {
    pub value: Vec<f32>,
    pub gx: Vec<f32>,
    pub gy: Vec<f32>,
    pub gz: Vec<f32>,
    coef: Vec<[f32; 8]>,
}

impl RowBuffers {
    pub fn new(nx: usize, waves: usize) -> Self {
        Self {
            value: vec![0.0; nx],
            gx: vec![0.0; nx],
            gy: vec![0.0; nx],
            gz: vec![0.0; nx],
            coef: vec![[0.0; 8]; waves],
        }
    }
}

impl WaveTables {
    pub fn new(noise: &WaveNoise, grid: &Grid) -> Self {
        let waves = noise.len();
        let [nx, ny, nz] = grid.dims;
        let table = |axis: usize, n: usize, with_phase: bool| {
            let mut re = vec![0f32; waves * n];
            let mut im = vec![0f32; waves * n];
            for w in 0..waves {
                let k = noise.k[w][axis] as f64;
                let ph = if with_phase { noise.phase[w] as f64 } else { 0.0 };
                for i in 0..n {
                    let (s, c) = (k * (grid.origin[axis] + i as f64 * grid.spacing[axis]) + ph).sin_cos();
                    re[w * n + i] = c as f32;
                    im[w * n + i] = s as f32;
                }
            }
            (re, im)
        };
        let (xr, xi) = table(0, nx, false);
        let (yr, yi) = table(1, ny, false);
        let (zr, zi) = table(2, nz, true);
        Self { waves, nx, ny, nz, xr, xi, yr, yi, zr, zi, k: noise.k.clone(), amp: noise.amp }
    }

    /// Evaluates the field on voxels `i0..i0 + len` of row `(j, k)` into
    /// `buf.value[..len]`.
    pub fn row_value(&self, j: usize, k: usize, i0: usize, len: usize, buf: &mut RowBuffers) {
        debug_assert!(j < self.ny && k < self.nz && i0 + len <= self.nx);
        for w in 0..self.waves {
            let (cr, ci) = self.yz(w, j, k);
            buf.coef[w][0] = self.amp * cr;
            buf.coef[w][1] = -self.amp * ci;
        }
        let out = &mut buf.value[..len];
        out.fill(0.0);
        dispatch_value(self, &buf.coef, i0, out);
    }

    /// Evaluates the field and its gradient on voxels `i0..i0 + len` of row
    /// `(j, k)` into the first `len` entries of the buffers.
    pub fn row_value_grad(&self, j: usize, k: usize, i0: usize, len: usize, buf: &mut RowBuffers) {
        debug_assert!(j < self.ny && k < self.nz && i0 + len <= self.nx);
        for w in 0..self.waves {
            let (cr, ci) = self.yz(w, j, k);
            let kv = self.k[w];
            // value    = amp * (xr*cr - xi*ci)
            // sin(th)  =        xr*ci + xi*cr ;  d/da = -amp * k_a * sin(th)
            buf.coef[w] = [
                self.amp * cr,
                -self.amp * ci,
                -self.amp * kv[0] * ci,
                -self.amp * kv[0] * cr,
                -self.amp * kv[1] * ci,
                -self.amp * kv[1] * cr,
                -self.amp * kv[2] * ci,
                -self.amp * kv[2] * cr,
            ];
        }
        let RowBuffers { value, gx, gy, gz, coef } = buf;
        let (value, gx, gy, gz) = (&mut value[..len], &mut gx[..len], &mut gy[..len], &mut gz[..len]);
        value.fill(0.0);
        gx.fill(0.0);
        gy.fill(0.0);
        gz.fill(0.0);
        dispatch_grad(self, coef, i0, value, gx, gy, gz);
    }

    /// `exp(i(ky*y_j)) * exp(i(kz*z_k + phi))` for wave `w`.
    #[inline]
    fn yz(&self, w: usize, j: usize, k: usize) -> (f32, f32) {
        let (ar, ai) = (self.yr[w * self.ny + j], self.yi[w * self.ny + j]);
        let (br, bi) = (self.zr[w * self.nz + k], self.zi[w * self.nz + k]);
        (ar * br - ai * bi, ar * bi + ai * br)
    }
}

// The kernels below are written as plain loops over equal-length slices so
// LLVM vectorises them. They are compiled twice: once for the baseline target
// and once with AVX2 enabled, selected at run time. No FMA contraction is
// used, so both paths give bit-identical results.

#[inline(always)]
fn kernel_value(t: &WaveTables, coef: &[[f32; 8]], i0: usize, out: &mut [f32]) {
    let n = out.len();
    for (w, c) in coef.iter().enumerate() {
        let xr = &t.xr[w * t.nx + i0..][..n];
        let xi = &t.xi[w * t.nx + i0..][..n];
        let (c0, c1) = (c[0], c[1]);
        for i in 0..n {
            out[i] += xr[i] * c0 + xi[i] * c1;
        }
    }
}

#[inline(always)]
fn kernel_grad(
    t: &WaveTables,
    coef: &[[f32; 8]],
    i0: usize,
    value: &mut [f32],
    gx: &mut [f32],
    gy: &mut [f32],
    gz: &mut [f32],
) {
    let n = value.len();
    let (gx, gy, gz) = (&mut gx[..n], &mut gy[..n], &mut gz[..n]);
    for (w, c) in coef.iter().enumerate() {
        let xr = &t.xr[w * t.nx + i0..][..n];
        let xi = &t.xi[w * t.nx + i0..][..n];
        for i in 0..n {
            let (a, b) = (xr[i], xi[i]);
            value[i] += a * c[0] + b * c[1];
            gx[i] += a * c[2] + b * c[3];
            gy[i] += a * c[4] + b * c[5];
            gz[i] += a * c[6] + b * c[7];
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn kernel_value_avx2(t: &WaveTables, coef: &[[f32; 8]], i0: usize, out: &mut [f32]) {
    kernel_value(t, coef, i0, out);
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn kernel_grad_avx2(
    t: &WaveTables,
    coef: &[[f32; 8]],
    i0: usize,
    value: &mut [f32],
    gx: &mut [f32],
    gy: &mut [f32],
    gz: &mut [f32],
) {
    kernel_grad(t, coef, i0, value, gx, gy, gz);
}

fn dispatch_value(t: &WaveTables, coef: &[[f32; 8]], i0: usize, out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // SAFETY: AVX2 support was just verified at run time.
        unsafe { kernel_value_avx2(t, coef, i0, out) };
        return;
    }
    kernel_value(t, coef, i0, out);
}

fn dispatch_grad(
    t: &WaveTables,
    coef: &[[f32; 8]],
    i0: usize,
    value: &mut [f32],
    gx: &mut [f32],
    gy: &mut [f32],
    gz: &mut [f32],
) {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx2") {
        // SAFETY: AVX2 support was just verified at run time.
        unsafe { kernel_grad_avx2(t, coef, i0, value, gx, gy, gz) };
        return;
    }
    kernel_grad(t, coef, i0, value, gx, gy, gz);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(z_elongation: f64) -> WaveNoise {
        WaveNoise::new(&mut Rng::new(3), 10.0, 96, 0.35, z_elongation, None)
    }

    #[test]
    fn unit_variance_and_zero_mean() {
        let n = noise(1.0);
        let mut rng = Rng::new(9);
        let (mut sum, mut sq) = (0.0f64, 0.0f64);
        let samples = 20_000;
        for _ in 0..samples {
            let p = [rng.uniform() * 400.0, rng.uniform() * 400.0, rng.uniform() * 400.0];
            let v = n.eval(p).0 as f64;
            sum += v;
            sq += v * v;
        }
        let mean = sum / samples as f64;
        let var = sq / samples as f64 - mean * mean;
        assert!(mean.abs() < 0.05, "mean {mean}");
        assert!((var - 1.0).abs() < 0.1, "var {var}");
    }

    #[test]
    fn analytic_gradient_matches_finite_differences() {
        let n = noise(2.0);
        let p = [3.1, -7.4, 12.9];
        let (_, g) = n.eval(p);
        let h = 1e-4;
        for a in 0..3 {
            let mut hi = p;
            let mut lo = p;
            hi[a] += h;
            lo[a] -= h;
            let fd = (n.eval(hi).0 as f64 - n.eval(lo).0 as f64) / (2.0 * h);
            assert!((fd - g[a] as f64).abs() < 2e-3, "axis {a}: {fd} vs {}", g[a]);
        }
    }

    #[test]
    fn rows_match_point_evaluation() {
        let n = noise(2.0);
        let grid = Grid { origin: [-2.0, 1.0, 5.0], spacing: [0.3; 3], dims: [37, 5, 4] };
        let tables = WaveTables::new(&n, &grid);
        let mut buf = RowBuffers::new(grid.nx(), n.len());
        let (j, k, i0, len) = (3, 2, 4, 29);
        tables.row_value_grad(j, k, i0, len, &mut buf);
        for i in 0..len {
            let (v, g) = n.eval([grid.x(i0 + i), grid.y(j), grid.z(k)]);
            assert!((buf.value[i] - v).abs() < 2e-4);
            assert!((buf.gx[i] - g[0]).abs() < 2e-4);
            assert!((buf.gy[i] - g[1]).abs() < 2e-4);
            assert!((buf.gz[i] - g[2]).abs() < 2e-4);
        }
        let reference: Vec<f32> = buf.value[..len].to_vec();
        tables.row_value(j, k, i0, len, &mut buf);
        for i in 0..len {
            assert!((buf.value[i] - reference[i]).abs() < 1e-6);
        }
    }

    #[test]
    fn z_elongation_turns_gradients_horizontal() {
        let iso = noise(1.0).grad_rms_axes();
        let tall = noise(2.0).grad_rms_axes();
        assert!((iso[2] / iso[0] - 1.0).abs() < 0.35);
        // Vertical gradient component halves: surfaces become steeper.
        assert!((tall[2] / iso[2] - 0.5).abs() < 1e-6);
        assert!((tall[0] / iso[0] - 1.0).abs() < 1e-6);
    }
}
