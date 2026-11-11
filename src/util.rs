//! Small dependency-free helpers: deterministic RNG, a fast hasher and a
//! pointer wrapper for writing disjoint regions of one buffer from many threads.

use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::marker::PhantomData;
use std::sync::OnceLock;

static TIME_SOURCE: OnceLock<fn() -> f64> = OnceLock::new();

/// Replaces the clock behind [`now_seconds`]. Needed where the standard
/// library has none, such as WebAssembly in a browser. Only the first call
/// has an effect.
pub fn set_time_source(source: fn() -> f64) {
    let _ = TIME_SOURCE.set(source);
}

/// Seconds on a monotonic clock with an arbitrary origin; only differences
/// are meaningful. Without a clock (see [`set_time_source`]) it returns 0, so
/// timings read as zero instead of failing.
pub fn now_seconds() -> f64 {
    if let Some(source) = TIME_SOURCE.get() {
        return source();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
    }
    #[cfg(target_arch = "wasm32")]
    {
        0.0
    }
}

/// xoshiro256++ seeded through SplitMix64.
///
/// The generator is implemented here (instead of pulling in `rand`) so that a
/// given `--seed` produces bit-identical lattices on every platform and
/// toolchain version.
#[derive(Clone, Debug)]
pub struct Rng {
    s: [u64; 4],
    spare_normal: Option<f64>,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        let s = [next(), next(), next(), next()];
        Self { s, spare_normal: None }
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[0].wrapping_add(self.s[3]).rotate_left(23).wrapping_add(self.s[0]);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Uniform in `[0, 1)`.
    #[inline]
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in `[0, n)`.
    #[inline]
    pub fn below(&mut self, n: usize) -> usize {
        debug_assert!(n > 0);
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    /// Standard normal sample (Box–Muller, polar-free form).
    pub fn normal(&mut self) -> f64 {
        if let Some(v) = self.spare_normal.take() {
            return v;
        }
        let u1 = (1.0 - self.uniform()).max(f64::MIN_POSITIVE);
        let u2 = self.uniform();
        let r = (-2.0 * u1.ln()).sqrt();
        let (s, c) = (std::f64::consts::TAU * u2).sin_cos();
        self.spare_normal = Some(r * s);
        r * c
    }
}

/// FxHash: a very fast non-cryptographic hasher, good for integer keys.
#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

const FX_SEED: u64 = 0x51_7C_C1_B7_27_22_0A_95;

impl FxHasher {
    #[inline]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rest) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        if !rest.is_empty() {
            let mut buf = [0u8; 8];
            buf[..rest.len()].copy_from_slice(rest);
            self.add(u64::from_le_bytes(buf));
        }
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add(i as u64);
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add(i);
    }
    #[inline]
    fn write_i64(&mut self, i: i64) {
        self.add(i as u64);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;

/// A slice that several threads may write to, as long as they touch disjoint
/// index ranges. Used for strided (non-contiguous) parallel passes that
/// `par_chunks_mut` cannot express, e.g. columns along Z.
#[derive(Clone, Copy)]
pub struct SharedSlice<'a, T> {
    ptr: *mut T,
    len: usize,
    _marker: PhantomData<&'a mut [T]>,
}

// SAFETY: the wrapper only hands out sub-slices through `unsafe` methods whose
// contract requires callers to keep concurrently used ranges disjoint.
unsafe impl<T: Send> Send for SharedSlice<'_, T> {}
unsafe impl<T: Send> Sync for SharedSlice<'_, T> {}

impl<'a, T> SharedSlice<'a, T> {
    pub fn new(slice: &'a mut [T]) -> Self {
        Self { ptr: slice.as_mut_ptr(), len: slice.len(), _marker: PhantomData }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// # Safety
    /// No other thread may read or write `start..start + len` while the
    /// returned slice is alive.
    #[inline]
    #[allow(clippy::mut_from_ref)]
    pub unsafe fn slice_mut(&self, start: usize, len: usize) -> &mut [T] {
        debug_assert!(start + len <= self.len);
        // SAFETY: bounds checked above (debug) and guaranteed by the caller;
        // exclusivity is the caller's contract.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.add(start), len) }
    }

    /// # Safety
    /// No other thread may write `start..start + len` while the returned
    /// slice is alive.
    #[inline]
    pub unsafe fn slice(&self, start: usize, len: usize) -> &[T] {
        debug_assert!(start + len <= self.len);
        // SAFETY: see `slice_mut`.
        unsafe { std::slice::from_raw_parts(self.ptr.add(start), len) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_is_deterministic_and_roughly_normal() {
        let mut a = Rng::new(7);
        let mut b = Rng::new(7);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
        let mut r = Rng::new(123);
        let n = 200_000;
        let (mut sum, mut sq) = (0.0, 0.0);
        for _ in 0..n {
            let v = r.normal();
            sum += v;
            sq += v * v;
        }
        let mean = sum / n as f64;
        let var = sq / n as f64 - mean * mean;
        assert!(mean.abs() < 0.01, "mean {mean}");
        assert!((var - 1.0).abs() < 0.02, "var {var}");
    }

    #[test]
    fn clock_moves_forward() {
        let a = now_seconds();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(now_seconds() > a);
    }

    #[test]
    fn below_stays_in_range() {
        let mut r = Rng::new(1);
        for _ in 0..10_000 {
            assert!(r.below(17) < 17);
        }
    }
}
