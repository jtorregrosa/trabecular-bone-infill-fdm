//! Scalar math helpers.

/// Inverse of the standard normal CDF (Acklam's rational approximation,
/// relative error below 1.2e-9 over the open unit interval).
#[allow(clippy::excessive_precision)] // coefficients quoted as published
pub fn ndtri(p: f64) -> f64 {
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_690e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] =
        [7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1, 2.445_134_137_142_996, 3.754_408_661_907_416];
    const P_LOW: f64 = 0.02425;

    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    let tail = |q: f64| {
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    };
    if p < P_LOW {
        tail((-2.0 * p.ln()).sqrt())
    } else if p <= 1.0 - P_LOW {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    }
}

/// Smooth maximum: `max(a, b)` plus a fillet of radius about `k` where the two
/// fields meet. Used as a smooth union of signed-distance-like fields.
#[inline]
pub fn smooth_max(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.max(b);
    }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.max(b) + h * h * k * 0.25
}

/// Hermite smoothstep, clamped to `[0, 1]`.
#[inline]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge1 <= edge0 {
        return if x >= edge1 { 1.0 } else { 0.0 };
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Linear-interpolated quantile of an unsorted sample (sorts in place).
pub fn quantile_in_place(values: &mut [f32], q: f64) -> f32 {
    assert!(!values.is_empty());
    values.sort_unstable_by(f32::total_cmp);
    let pos = q.clamp(0.0, 1.0) * (values.len() - 1) as f64;
    let lo = pos.floor() as usize;
    let hi = (lo + 1).min(values.len() - 1);
    let t = (pos - lo as f64) as f32;
    values[lo] + (values[hi] - values[lo]) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndtri_matches_known_values() {
        assert!(ndtri(0.5).abs() < 1e-12);
        assert!((ndtri(0.975) - 1.959_963_984_540_054).abs() < 1e-7);
        assert!((ndtri(0.841_344_746_068_543) - 1.0).abs() < 1e-7);
        assert!((ndtri(0.001) + 3.090_232_306_167_813).abs() < 1e-7);
        assert!((ndtri(0.3) + ndtri(0.7)).abs() < 1e-9);
    }

    #[test]
    fn smooth_max_bounds() {
        assert_eq!(smooth_max(1.0, -5.0, 0.5), 1.0);
        let v = smooth_max(1.0, 1.0, 1.0);
        assert!((v - 1.25).abs() < 1e-6);
        assert_eq!(smooth_max(2.0, 3.0, 0.0), 3.0);
    }

    #[test]
    fn quantiles() {
        let mut v = vec![4.0, 1.0, 3.0, 2.0, 5.0];
        assert_eq!(quantile_in_place(&mut v, 0.5), 3.0);
        assert_eq!(quantile_in_place(&mut v, 0.0), 1.0);
        assert_eq!(quantile_in_place(&mut v, 1.0), 5.0);
    }
}
