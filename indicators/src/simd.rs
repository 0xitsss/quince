#[cfg(all(target_feature = "avx2", feature = "simd"))]
use core::arch::x86_64::*;

#[cfg(all(target_feature = "avx2", feature = "simd"))]
#[inline(always)]
unsafe fn reduce_add_pd(v: __m256d) -> f64 {
    let hi128 = _mm256_extractf128_pd(v, 1);
    let lo128 = _mm256_castpd256_pd128(v);
    let s128 = _mm_add_pd(lo128, hi128);
    let swp = _mm_shuffle_pd(s128, s128, 0b01);
    _mm_cvtsd_f64(_mm_add_sd(s128, swp))
}

#[cfg(all(target_feature = "avx2", feature = "simd"))]
#[inline(always)]
unsafe fn reduce_max_pd(v: __m256d) -> f64 {
    let hi128 = _mm256_extractf128_pd(v, 1);
    let lo128 = _mm256_castpd256_pd128(v);
    let m128 = _mm_max_pd(lo128, hi128);
    let swp = _mm_shuffle_pd(m128, m128, 0b01);
    _mm_cvtsd_f64(_mm_max_sd(m128, swp))
}

#[cfg(all(target_feature = "avx2", feature = "simd"))]
#[inline(always)]
unsafe fn reduce_min_pd(v: __m256d) -> f64 {
    let hi128 = _mm256_extractf128_pd(v, 1);
    let lo128 = _mm256_castpd256_pd128(v);
    let m128 = _mm_min_pd(lo128, hi128);
    let swp = _mm_shuffle_pd(m128, m128, 0b01);
    _mm_cvtsd_f64(_mm_min_sd(m128, swp))
}

#[allow(dead_code)]
fn sum_scalar(a: &[f64], b: &[f64]) -> f64 {
    let mut s = 0.0;
    for &v in a {
        s += v;
    }
    for &v in b {
        s += v;
    }
    s
}

#[inline]
pub fn sum(a: &[f64], b: &[f64]) -> f64 {
    #[cfg(all(target_feature = "avx2", feature = "simd"))]
    return sum_avx2(a, b);
    #[cfg(not(all(target_feature = "avx2", feature = "simd")))]
    sum_scalar(a, b)
}

#[inline]
pub fn sum_sq_diff(a: &[f64], b: &[f64], center: f64) -> f64 {
    #[cfg(all(target_feature = "avx2", feature = "simd"))]
    return sum_sq_diff_avx2(a, b, center);
    #[cfg(not(all(target_feature = "avx2", feature = "simd")))]
    sum_sq_diff_scalar(a, b, center)
}

#[cfg(all(target_feature = "avx2", feature = "simd"))]
fn sum_avx2(a: &[f64], b: &[f64]) -> f64 {
    unsafe {
        let mut acc = _mm256_setzero_pd();
        let mut tail = 0.0_f64;
        for slice in [a, b] {
            let n = slice.len() / 4 * 4;
            let (full, rem) = slice.split_at(n);
            for chunk in full.chunks_exact(4) {
                acc = _mm256_add_pd(acc, _mm256_loadu_pd(chunk.as_ptr()));
            }
            for &v in rem {
                tail += v;
            }
        }
        reduce_add_pd(acc) + tail
    }
}

#[allow(dead_code)]
fn weighted_sum_scalar(a: &[f64], b: &[f64], start: usize) -> f64 {
    let mut s = 0.0;
    let mut w = start;
    for &v in a {
        s += v * w as f64;
        w += 1;
    }
    for &v in b {
        s += v * w as f64;
        w += 1;
    }
    s
}

#[inline]
pub fn weighted_sum(a: &[f64], b: &[f64], start: usize) -> f64 {
    #[cfg(all(target_feature = "avx2", feature = "simd"))]
    return weighted_sum_avx2(a, b, start);
    #[cfg(not(all(target_feature = "avx2", feature = "simd")))]
    weighted_sum_scalar(a, b, start)
}

#[cfg(all(target_feature = "avx2", feature = "simd"))]
fn weighted_sum_avx2(a: &[f64], b: &[f64], start: usize) -> f64 {
    unsafe {
        let mut acc = _mm256_setzero_pd();
        let mut tail = 0.0_f64;
        let mut weight = start;
        let inc = _mm256_set1_pd(4.0);
        for slice in [a, b] {
            let n = slice.len() / 4 * 4;
            let (full, rem) = slice.split_at(n);
            if !full.is_empty() {
                let mut w = _mm256_set_pd(
                    (weight + 3) as f64,
                    (weight + 2) as f64,
                    (weight + 1) as f64,
                    weight as f64,
                );
                for chunk in full.chunks_exact(4) {
                    let v = _mm256_loadu_pd(chunk.as_ptr());
                    acc = _mm256_fmadd_pd(v, w, acc);
                    w = _mm256_add_pd(w, inc);
                }
                weight += full.len();
            }
            for &v in rem {
                tail += v * weight as f64;
                weight += 1;
            }
        }
        reduce_add_pd(acc) + tail
    }
}

#[allow(dead_code)]
fn sum_and_sum_xy_scalar(a: &[f64], b: &[f64]) -> (f64, f64) {
    let mut sum_y = 0.0;
    let mut sum_xy = 0.0;
    let mut i = 0_usize;
    for &v in a {
        sum_y += v;
        sum_xy += v * i as f64;
        i += 1;
    }
    for &v in b {
        sum_y += v;
        sum_xy += v * i as f64;
        i += 1;
    }
    (sum_y, sum_xy)
}

#[inline]
pub fn sum_and_sum_xy(a: &[f64], b: &[f64]) -> (f64, f64) {
    #[cfg(all(target_feature = "avx2", feature = "simd"))]
    return sum_and_sum_xy_avx2(a, b);
    #[cfg(not(all(target_feature = "avx2", feature = "simd")))]
    sum_and_sum_xy_scalar(a, b)
}

#[cfg(all(target_feature = "avx2", feature = "simd"))]
fn sum_and_sum_xy_avx2(a: &[f64], b: &[f64]) -> (f64, f64) {
    unsafe {
        let mut acc_sum = _mm256_setzero_pd();
        let mut acc_xy = _mm256_setzero_pd();
        let mut tail_sum = 0.0_f64;
        let mut tail_xy = 0.0_f64;
        let mut idx = 0_usize;
        let inc = _mm256_set1_pd(4.0);
        for slice in [a, b] {
            let n = slice.len() / 4 * 4;
            let (full, rem) = slice.split_at(n);
            if !full.is_empty() {
                let offset = _mm256_set1_pd(idx as f64);
                let mut w = _mm256_add_pd(_mm256_set_pd(3.0, 2.0, 1.0, 0.0), offset);
                for chunk in full.chunks_exact(4) {
                    let v = _mm256_loadu_pd(chunk.as_ptr());
                    acc_sum = _mm256_add_pd(acc_sum, v);
                    acc_xy = _mm256_fmadd_pd(v, w, acc_xy);
                    w = _mm256_add_pd(w, inc);
                }
                idx += full.len();
            }
            for &v in rem {
                tail_sum += v;
                tail_xy += v * idx as f64;
                idx += 1;
            }
        }
        (
            reduce_add_pd(acc_sum) + tail_sum,
            reduce_add_pd(acc_xy) + tail_xy,
        )
    }
}

#[allow(dead_code)]
fn sum_abs_diff_scalar(a: &[f64], b: &[f64], center: f64) -> f64 {
    let mut s = 0.0;
    for &v in a {
        s += (v - center).abs();
    }
    for &v in b {
        s += (v - center).abs();
    }
    s
}

#[inline]
pub fn sum_abs_diff(a: &[f64], b: &[f64], center: f64) -> f64 {
    #[cfg(all(target_feature = "avx2", feature = "simd"))]
    return sum_abs_diff_avx2(a, b, center);
    #[cfg(not(all(target_feature = "avx2", feature = "simd")))]
    sum_abs_diff_scalar(a, b, center)
}

#[cfg(all(target_feature = "avx2", feature = "simd"))]
fn sum_abs_diff_avx2(a: &[f64], b: &[f64], center: f64) -> f64 {
    unsafe {
        let mut acc = _mm256_setzero_pd();
        let mut tail = 0.0_f64;
        let c = _mm256_set1_pd(center);
        for slice in [a, b] {
            let n = slice.len() / 4 * 4;
            let (full, rem) = slice.split_at(n);
            for chunk in full.chunks_exact(4) {
                let v = _mm256_loadu_pd(chunk.as_ptr());
                let diff = _mm256_sub_pd(v, c);
                let neg = _mm256_sub_pd(_mm256_setzero_pd(), diff);
                let absv = _mm256_max_pd(diff, neg);
                acc = _mm256_add_pd(acc, absv);
            }
            for &v in rem {
                tail += (v - center).abs();
            }
        }
        reduce_add_pd(acc) + tail
    }
}

#[allow(dead_code)]
fn min_max_scalar(a: &[f64], b: &[f64]) -> (f64, f64) {
    let mut min_val = f64::INFINITY;
    let mut max_val = f64::NEG_INFINITY;
    for &v in a {
        if v < min_val {
            min_val = v;
        }
        if v > max_val {
            max_val = v;
        }
    }
    for &v in b {
        if v < min_val {
            min_val = v;
        }
        if v > max_val {
            max_val = v;
        }
    }
    (min_val, max_val)
}

#[inline]
pub fn min_max(a: &[f64], b: &[f64]) -> (f64, f64) {
    #[cfg(all(target_feature = "avx2", feature = "simd"))]
    return min_max_avx2(a, b);
    #[cfg(not(all(target_feature = "avx2", feature = "simd")))]
    min_max_scalar(a, b)
}

#[cfg(all(target_feature = "avx2", feature = "simd"))]
fn min_max_avx2(a: &[f64], b: &[f64]) -> (f64, f64) {
    unsafe {
        let mut max_vec = _mm256_set1_pd(f64::NEG_INFINITY);
        let mut min_vec = _mm256_set1_pd(f64::INFINITY);
        for slice in [a, b] {
            let n = slice.len() / 4 * 4;
            let (full, rem) = slice.split_at(n);
            for chunk in full.chunks_exact(4) {
                let v = _mm256_loadu_pd(chunk.as_ptr());
                max_vec = _mm256_max_pd(max_vec, v);
                min_vec = _mm256_min_pd(min_vec, v);
            }
            for &x in rem {
                max_vec = _mm256_max_pd(max_vec, _mm256_set1_pd(x));
                min_vec = _mm256_min_pd(min_vec, _mm256_set1_pd(x));
            }
        }
        (reduce_min_pd(min_vec), reduce_max_pd(max_vec))
    }
}

#[allow(dead_code)]
fn sum_sq_diff_scalar(a: &[f64], b: &[f64], center: f64) -> f64 {
    let mut s = 0.0;
    for &v in a {
        let d = v - center;
        s += d * d;
    }
    for &v in b {
        let d = v - center;
        s += d * d;
    }
    s
}

#[cfg(all(target_feature = "avx2", feature = "simd"))]
fn sum_sq_diff_avx2(a: &[f64], b: &[f64], center: f64) -> f64 {
    unsafe {
        let mut acc = _mm256_setzero_pd();
        let mut tail = 0.0_f64;
        let c = _mm256_set1_pd(center);
        for slice in [a, b] {
            let n = slice.len() / 4 * 4;
            let (full, rem) = slice.split_at(n);
            for chunk in full.chunks_exact(4) {
                let v = _mm256_loadu_pd(chunk.as_ptr());
                let diff = _mm256_sub_pd(v, c);
                acc = _mm256_fmadd_pd(diff, diff, acc);
            }
            for &x in rem {
                let d = x - center;
                tail += d * d;
            }
        }
        reduce_add_pd(acc) + tail
    }
}
