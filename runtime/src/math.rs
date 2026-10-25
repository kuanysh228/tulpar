//! Математика (libm жоқ): бүтін/нақты дәреже, fmod. Элементарлы функциялар өз іске асыруымызда.

use crate::sys::*;

#[unsafe(no_mangle)]
pub extern "C" fn tlp_pow_i(mut a: i64, mut b: i64) -> i64 {
    if b < 0 {
        fatal("бүтін санды теріс дәрежеге шығаруға болмайды (`нақты` қолданыңыз)".as_bytes());
    }
    let mut r: i64 = 1;
    while b > 0 {
        if b & 1 == 1 {
            r = r.wrapping_mul(a);
        }
        a = a.wrapping_mul(a);
        b >>= 1;
    }
    r
}

pub fn floor(x: f64) -> f64 {
    if !(x.abs() < 4503599627370496.0) {
        return x;
    }
    let i = x as i64 as f64;
    if i > x { i - 1.0 } else { i }
}

fn frexp(x: f64) -> (f64, i32) {
    // x > 0, қалыпты
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32;
    if e == 0 {
        // денормал
        let (m, e2) = frexp(x * 18014398509481984.0);
        return (m, e2 - 54);
    }
    let m = f64::from_bits((bits & 0x800f_ffff_ffff_ffff) | (1022u64 << 52));
    (m, e - 1022)
}

pub fn ln(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 {
        return f64::NEG_INFINITY;
    }
    if x.is_infinite() {
        return x;
    }
    let (mut m, mut e) = frexp(x);
    if m < core::f64::consts::FRAC_1_SQRT_2 {
        m *= 2.0;
        e -= 1;
    }
    let s = (m - 1.0) / (m + 1.0);
    let s2 = s * s;
    let mut term = s;
    let mut sum = 0.0;
    let mut k = 1.0;
    for _ in 0..30 {
        sum += term / k;
        term *= s2;
        k += 2.0;
    }
    2.0 * sum + e as f64 * core::f64::consts::LN_2
}

pub fn exp(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x > 709.78 {
        return f64::INFINITY;
    }
    if x < -745.2 {
        return 0.0;
    }
    let k = floor(x / core::f64::consts::LN_2 + 0.5);
    let r = x - k * core::f64::consts::LN_2;
    let mut term = 1.0;
    let mut sum = 1.0;
    for i in 1..=20 {
        term *= r / i as f64;
        sum += term;
    }
    let mut ki = k as i32;
    let mut res = sum;
    // 2^k
    while ki > 1000 {
        res *= f64::from_bits((1023u64 + 1000) << 52);
        ki -= 1000;
    }
    while ki < -1000 {
        res *= f64::from_bits((1023u64 - 1000) << 52);
        ki += 1000;
    }
    res * f64::from_bits(((1023 + ki) as u64) << 52)
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_pow_f(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        return 1.0;
    }
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    let bi = b as i64;
    if bi as f64 == b && b.abs() < 1e9 {
        // бүтін дәреже: квадраттау
        let mut n = bi.unsigned_abs();
        let mut base = a;
        let mut r = 1.0;
        while n > 0 {
            if n & 1 == 1 {
                r *= base;
            }
            base *= base;
            n >>= 1;
        }
        return if bi < 0 { 1.0 / r } else { r };
    }
    if a < 0.0 {
        return f64::NAN;
    }
    if a == 0.0 {
        return if b > 0.0 { 0.0 } else { f64::INFINITY };
    }
    exp(b * ln(a))
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_fmod(a: f64, b: f64) -> f64 {
    if b == 0.0 {
        fatal("нөлге бөлу".as_bytes());
    }
    let q = floor(a / b);
    a - q * b
}

const PI: f64 = core::f64::consts::PI;
const HALF_PI: f64 = core::f64::consts::FRAC_PI_2;

/// sin үшін Тейлор қатары, |x| <= pi/4
fn sin_k(x: f64) -> f64 {
    let x2 = x * x;
    let mut term = x;
    let mut sum = x;
    let mut n = 1.0;
    for _ in 0..12 {
        term *= -x2 / ((n + 1.0) * (n + 2.0));
        sum += term;
        n += 2.0;
    }
    sum
}

fn cos_k(x: f64) -> f64 {
    let x2 = x * x;
    let mut term = 1.0;
    let mut sum = 1.0;
    let mut n = 0.0;
    for _ in 0..12 {
        term *= -x2 / ((n + 1.0) * (n + 2.0));
        sum += term;
        n += 2.0;
    }
    sum
}

/// (квадрант, қалдық r): x = q*pi/2 + r, |r| <= pi/4
fn reduce(x: f64) -> (i64, f64) {
    let q = floor(x / HALF_PI + 0.5);
    // Коди–Уэйт: pi/2-ні екі бөлікке бөлеміз
    const P1: f64 = 1.5707963267341256;
    const P2: f64 = 6.077100506506192e-11;
    let r = (x - q * P1) - q * P2;
    (q as i64, r)
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_sin(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (q, r) = reduce(x);
    match q.rem_euclid(4) {
        0 => sin_k(r),
        1 => cos_k(r),
        2 => -sin_k(r),
        _ => -cos_k(r),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_cos(x: f64) -> f64 {
    if !x.is_finite() {
        return f64::NAN;
    }
    let (q, r) = reduce(x);
    match q.rem_euclid(4) {
        0 => cos_k(r),
        1 => -sin_k(r),
        2 => -cos_k(r),
        _ => sin_k(r),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_tan(x: f64) -> f64 {
    tlp_sin(x) / tlp_cos(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_atan(x: f64) -> f64 {
    if x.is_nan() {
        return x;
    }
    if x < 0.0 {
        return -tlp_atan(-x);
    }
    if x > 1e300 {
        return HALF_PI;
    }
    if x > 1.0 {
        return HALF_PI - tlp_atan(1.0 / x);
    }
    // x in [0,1]: arg азайту: atan(x) = 2*atan(x / (1 + sqrt(1+x^2)))
    let mut y = x;
    for _ in 0..3 {
        y = y / (1.0 + sqrt(1.0 + y * y));
    }
    // қатар: y - y^3/3 + ...
    let y2 = y * y;
    let mut term = y;
    let mut sum = y;
    let mut k = 1.0;
    for _ in 0..14 {
        term *= -y2;
        k += 2.0;
        sum += term / k;
    }
    sum * 8.0
}

fn sqrt(x: f64) -> f64 {
    let r: f64;
    unsafe {
        core::arch::asm!("sqrtsd {0}, {1}", out(xmm_reg) r, in(xmm_reg) x, options(pure, nomem, nostack));
    }
    r
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_atan2(y: f64, x: f64) -> f64 {
    if x > 0.0 {
        tlp_atan(y / x)
    } else if x < 0.0 {
        if y >= 0.0 { tlp_atan(y / x) + PI } else { tlp_atan(y / x) - PI }
    } else if y > 0.0 {
        HALF_PI
    } else if y < 0.0 {
        -HALF_PI
    } else {
        0.0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tlp_exp(x: f64) -> f64 {
    exp(x)
}
#[unsafe(no_mangle)]
pub extern "C" fn tlp_ln(x: f64) -> f64 {
    ln(x)
}
#[unsafe(no_mangle)]
pub extern "C" fn tlp_log10(x: f64) -> f64 {
    ln(x) / core::f64::consts::LN_10
}
