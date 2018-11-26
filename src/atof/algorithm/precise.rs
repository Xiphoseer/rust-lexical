//! Correct algorithms for string-to-float conversions.
//!
//! This implementation is loosely based off the Golang implementation,
//! found here:
//!     https://golang.org/src/strconv/atof.go
//!
//! The extended-precision and decimal versions are highly
// Fix a compiler bug that thinks `ExactExponent` isn't used.
#![allow(unused_imports)]

// BENCHMARKS
//  The Python benchmarks were done by pre-parsing the data into a Python
//  list, and then using the following code:
//  ```text
//  %%timeit
//  for i in l:
//      float(i)
//  ```
//
// The C++ benchmarks (libstdc++) were done using the following code:
//
//  ```text
//  #include <benchmark/benchmark.h>
//  #include <cstdlib>
//
//  static void bench(benchmark::State& state)
//  {
//      for (auto _ : state) {
//          for (auto value: DATA) {
//              benchmark::DoNotOptimize(std::strtod(value, nullptr));
//          }
//      }
//  }
//  BENCHMARK(bench);
//  BENCHMARK_MAIN();
//  ```
//
// The Go benchmarks were done using the following code:
//
//  ```text
//  package parse
//
//  import "strconv"
//  import "testing"
//
//  var result float64
//
//  func Benchmark(b *testing.B) {
//      var r float64
//      for n := 0; n < b.N; n++ {
//          for _, v := range data {
//              if f, err := strconv.ParseFloat(v, 64); err == nil {
//                  r = f
//              }
//          }
//      }
//      result = r
//  }
//  ```

// Code the generate the benchmark plot for f32:
//  import numpy as np
//  import pandas as pd
//  import matplotlib.pyplot as plt
//  plt.style.use('ggplot')
//  lexical = np.array([176665, 607635, 832949, 867837, 1148750, 912004, 1460000]) / 1e6
//  parse = np.array([198785, 10206859, 27092915, 90488388, 250777648, 29324566, 128600000]) / 1e6
//  index = ["2", "8", "16", "32", "64", "random", "malicious"]
//  df = pd.DataFrame({'lexical': lexical, 'parse': parse}, index = index)
//  ax = df.plot.bar(rot=0)
//  ax.set_yscale('log')
//  ax.set_ylabel("ms/iter")
//  ax.figure.tight_layout()
//  plt.show()

// Code the generate the benchmark plot for f64:
//  import numpy as np
//  import pandas as pd
//  import matplotlib.pyplot as plt
//  plt.style.use('ggplot')
//  lexical = np.array([175465, 335195, 757436, 977333, 1281781, 968977, 4250000]) / 1e6
//  parse = np.array([203726, 373929, 2770159, 4201497, 7961527, 133370247, 5856740000]) / 1e6
//  python = np.array([1760000, 1910000, 4230000, 6070000, 7030000, 6540000, 268000000]) / 1e6
//  cpp = np.array([970611, 1254091, 1601604, 1976854, 2329004, 3111764, 4160000]) / 1e6
//  go = np.array([258830, 508871, 1437397, 2535061, 4476364, 2104273, 68840000]) / 1e6
//  index = ["2", "8", "16", "32", "64", "random", "malicious"]
//  df = pd.DataFrame({'lexical': lexical, 'parse': parse, 'python': python, 'c++': cpp, 'go': go}, index = index)
//  ax = df.plot.bar(rot=0)
//  ax.set_yscale('log')
//  ax.set_ylabel("ms/iter")
//  ax.figure.tight_layout()
//  plt.show()

use atoi;
use float::*;
use table::*;
use util::*;
use super::bigfloat::Bigfloat;
use super::cached::CachedPowers;
use super::exponent::*;

// SHARED

// Fast path for the parse algorithm.
// In this case, the mantissa can be represented by an integer,
// which allows any value to be exactly reconstructed.

// PARSE
// -----

/// Parse the mantissa from a string.
///
/// Returns the mantissa, the shift in the mantissa relative to the dot,
/// a pointer to the current buffer position, and if the mantissa was
/// truncated.
///
/// The float string must be non-special, non-zero, and positive.
#[inline]
pub(super) unsafe extern "C" fn parse_mantissa<M>(base: u32, mut first: *const u8, last: *const u8)
    -> (M, i32, *const u8, bool)
    where M: Mantissa
{
    // Trim the leading 0s.
    // Need to force this here, since if not, conversion of usize dot to
    // i32 may truncate when mantissa does not, which would lead to faulty
    // results. If we trim the 0s here, we guarantee any time `dot as i32`
    // leads to a truncation, mantissa will overflow.
    first = ltrim_char_range(first, last, b'0');

    // Parse the integral value.
    // Use the checked parsers so the truncated value is valid even if
    // the entire value is not parsed.
    let mut mantissa: M = M::ZERO;
    let (f, truncated) = atoi::checked(&mut mantissa, base, first, last);

    // Check for trailing digits
    let has_fraction = distance(f, last) >= 1 && *f == b'.';
    if has_fraction && truncated == 0 {
        // Has a decimal, calculate the rest of it.
        let f = f.add(1);
        let tup = match mantissa.is_zero() {
            true  => {
                // Can ignore the leading digits while the mantissa is 0.
                // This allows us to represent extremely small values
                // using the fast route in non-scientific notation.
                // For example, this allows us to use the fast path for
                // both "1e-29" and "0.0000000000000000000000000001",
                // otherwise, only the former would work.
                let f = ltrim_char_range(f, last, b'0');
                atoi::checked(&mut mantissa, base, f, last)
            },
            false => atoi::checked(&mut mantissa, base, f, last),
        };
        // Subtract the number of truncated digits from the dot shift, since these
        // truncated digits are reflected in the distance but not in the mantissa.
        let dot_shift = distance(f, tup.0).try_i32_or_max() - tup.1.try_i32_or_max();
        (mantissa, dot_shift, tup.0, tup.1 != 0)
    } else if has_fraction {
        // Integral overflow occurred, cannot add more values, but a fraction exists.
        // Ignore the remaining characters, but factor them into the dot exponent.
        let f = f.add(1);
        let mut p = f;
        while p < last && (char_to_digit(*p) as u32) < base {
            p = p.add(1);
        }
        // Any truncated digits did not increase the mantissa, make dot_shift
        // negative to compensate.
        let dot_shift = -truncated.try_i32_or_max();
        (mantissa, dot_shift, p, true)
    } else {
        // No decimal, just return, noting if truncation occurred.
        // Any truncated digits did not increase the mantissa, make dot_shift
        // negative to compensate.
        let dot_shift = -truncated.try_i32_or_max();
        (mantissa, dot_shift, f, truncated != 0)
    }
}

/// Normalize the mantissa to check if it can use the fast-path.
///
/// Move digits from the mantissa to the exponent when possible.
#[inline]
pub(super) extern "C" fn normalize_mantissa<M>(mut mantissa: M, base: u32, mut exponent: i32)
    -> (M, i32)
    where M: Mantissa
{
    let base: M = as_cast(base);
    let base2 = base * base;
    let base4 = base2 * base2;

    // Use power-reduction, we're likely never going to enter most of these
    // loops, but it minimizes the number of expensive operations we need
    // to do.
    while mantissa >= base4 && (mantissa % base4).is_zero() {
        mantissa /= base4;
        exponent += 4;
    }
    while mantissa >= base2 && (mantissa % base2).is_zero() {
        mantissa /= base2;
        exponent += 2;
    }
    if (mantissa % base).is_zero() {
        mantissa /= base;
        exponent += 1;
    }
    (mantissa, exponent)
}

/// Parse the mantissa and exponent from a string.
///
/// Returns the mantissa, the exponent, number of digits since the dot
/// was seen, a pointer to the current buffer position, and if mantissa
/// was truncated.
///
/// The number of digits ignored relative to the dot may be positive
/// (digits past the dot added to the mantissa) or negative (truncated
/// digits from the integer component).
///
/// The float string must be non-special, non-zero, and positive.
#[inline]
unsafe extern "C" fn parse_float<M>(base: u32, first: *const u8, last: *const u8)
    -> (M, i32, *const u8, bool)
    where M: Mantissa
{
    let (mantissa, dot_shift, p, truncated) = parse_mantissa::<M>(base, first, last);
    let (exponent, p) = parse_exponent(base, p, last);
    let exponent = normalize_exponent(exponent, dot_shift);
    let (mantissa, exponent) = normalize_mantissa::<M>(mantissa, base, exponent);
    (mantissa, exponent, p, truncated)
}

// EXACT
// -----

/// Check if value is power of 2 and get the power.
#[inline]
fn pow2_exponent(base: u32) -> i32 {
    match base {
        2  => 1,
        4  => 2,
        8  => 3,
        16 => 4,
        32 => 5,
        _  => 0,
    }
}

/// Detect if a value is exactly halfway.
#[inline]
fn is_halfway<F: Float>(mantissa: u64)
    -> bool
{
    // Get the leading and trailing zeros from the least-significant bit.
    let leading_zeros: i32 = as_cast(64 - mantissa.leading_zeros());
    let trailing_zeros: i32 = as_cast(mantissa.trailing_zeros());

    // We need exactly mantissa+2 elements between these if it is halfway.
    // The hidden bit is mantissa+1 elements away, which is the last non-
    // truncated bit, while mantissa+2
    leading_zeros - trailing_zeros == F::MANTISSA_SIZE + 2
}

/// Convert power-of-two to exact value.
///
/// We will always get an exact representation.
///
/// This works since multiplying by the exponent will not affect the
/// mantissa unless the exponent is denormal, which will cause truncation
/// regardless.
#[inline]
fn pow2_to_exact<F: StablePower>(mantissa: u64, base: u32, pow2_exp: i32, exponent: i32)
    -> F
{
    debug_assert!(pow2_exp != 0, "Not a power of 2.");

    // As long as the value is within the bounds, we can get an exact value.
    // Since any power of 2 only affects the exponent, we should be able to get
    // any exact value.

    // We know that if any value is > than max_exp, we get infinity, since
    // the mantissa must be positive. We know that the actual value that
    // causes underflow is 64, use 65 since that prevents inaccurate
    // rounding for any pow2_exp.
    let (min_exp, max_exp) = F::exponent_limit(base);
    let underflow_exp = min_exp - (65 / pow2_exp);
    if exponent > max_exp {
        F::INFINITY
    } else if exponent < underflow_exp{
        F::ZERO
    } else if exponent < min_exp {
        // We know the mantissa is somewhere <= 65 below min_exp.
        // May still underflow, but it's close. Use the first multiplication
        // which guarantees no truncation, and then the second multiplication
        // which will round to the accurate representation.
        let remainder = exponent - min_exp;
        let float: F = as_cast(mantissa);
        let float = unsafe { float.pow2(pow2_exp * remainder).pow2(pow2_exp * min_exp) };
        float
    } else {
        let float: F = as_cast(mantissa);
        let float = unsafe { float.pow2(pow2_exp * exponent) };
        float
    }
}


/// Convert mantissa to exact value for a non-base2 power.
///
/// Returns the resulting float and if the value can be represented exactly.
#[inline]
fn to_exact<F: StablePower>(mantissa: u64, base: u32, exponent: i32) -> (F, bool)
{
    // logic error, disable in release builds
    debug_assert!(base >= 2 && base <= 36, "Numerical base must be from 2-36");
    debug_assert!(pow2_exponent(base) == 0, "Cannot use `to_exact` with a power of 2.");

    // `mantissa >> F::MANTISSA_SIZE != 0` effectively checks if the value
    // has a no bits above the hidden bit, which is what we want.
    let (min_exp, max_exp) = F::exponent_limit(base);
    if mantissa >> F::MANTISSA_SIZE != 0 {
        // Would require truncation of the mantissa.
        (F::ZERO, false)
    } else {
        let float: F = as_cast(mantissa);
        if exponent == 0 {
            // 0 exponent, same as value, exact representation.
            (float,  true)
        } else if exponent >= min_exp && exponent <= max_exp {
            // Value can be exactly represented, return the value.
            let float = unsafe { float.pow(base, exponent) };
            (float, true)
        } else {
            // Cannot be exactly represented, exponent multiplication
            // would require truncation.
            (F::ZERO, false)
        }
    }
}

// EXTENDED
// --------

// Moderate/slow path for the parse algorithm.
// In this case, the mantissa can be (partially) represented by an integer,
// however, the exponent or mantissa cannot be fully represented without
// truncating bytes. The moderate path uses a 64-bit integer, while
// the slow path uses a 128-bit integer.

// EXTENDED

pub trait FloatErrors: Mantissa {
    /// Get the full error scale.
    fn error_scale() -> u32;
    /// Get the half error scale.
    fn error_halfscale() -> u32;
    /// Determine if the number of errors is tolerable for float precision.
    fn error_is_accurate<F: Float>(count: u32, fp: &ExtendedFloat<Self>) -> bool;
}

impl FloatErrors for u64 {
    #[inline(always)]
    fn error_scale() -> u32 {
        8
    }

    #[inline(always)]
    fn error_halfscale() -> u32 {
        u64::error_scale() / 2
    }

    #[inline]
    fn error_is_accurate<F: Float>(count: u32, fp: &ExtendedFloat<u64>) -> bool
    {
        // Determine if extended-precision float is a good approximation.
        // If the error has affected too many units, the float will be
        // inaccurate, or if the representation is too close to halfway
        // that any operations could affect this halfway representation.
        // See the documentation for Bigfloat for more information.
        let bias = -(F::EXPONENT_BIAS - F::MANTISSA_SIZE);
        let denormal_exp = bias - 63;
        // This is always a valid u32, since (denormal_exp - fp.exp)
        // will always be positive and the significand size is {23, 52}.
        let extrabits = match fp.exp <= denormal_exp {
            true  => 64 - F::MANTISSA_SIZE + denormal_exp - fp.exp,
            false => 63 - F::MANTISSA_SIZE,
        };

        if extrabits > 65 {
            // Underflow, we have a literal 0.
            true
        } else if extrabits == 65 {
            // Underflow, we have a shift larger than the mantissa.
            // Representation is valid **only** if the value is close enough
            // overflow to the next bit within errors. If it overflows,
            // the representation is **not** valid.
            !fp.frac.overflowing_add(as_cast(count)).1
        } else {
            // Do a signed comparison, which will always be valid.
            let mask: u64 = lower_n_mask(extrabits.as_u64());
            let halfway: u64 = lower_n_halfway(extrabits.as_u64());
            let extra: u64 = fp.frac & mask;
            let errors: u64 = as_cast(count);
            let cmp1 = halfway.as_i64().wrapping_sub(errors.as_i64()) < extra.as_i64();
            let cmp2 = extra.as_i64() < halfway.as_i64().wrapping_add(errors.as_i64());

            // If both comparisons are true, we have significant rounding error,
            // and the value cannot be exactly represented. Otherwise, the
            // representation is valid.
            !(cmp1 && cmp2)
        }
    }
}

// 128-bit representation is always accurate, ignore this.
impl FloatErrors for u128 {
    #[inline(always)]
    fn error_scale() -> u32 {
        0
    }

    #[inline(always)]
    fn error_halfscale() -> u32 {
        0
    }

    #[inline]
    fn error_is_accurate<F: Float>(_: u32, _: &ExtendedFloat<u128>) -> bool {
        // Ignore the halfway problem, use more bits to aim for accuracy,
        // but short-circuit to avoid extremely slow operations.
        true
    }
}

/// Multiply the floating-point by the exponent.
///
/// Multiply by pre-calculated powers of the base, modify the extended-
/// float, and return if new value and if the value can be represented
/// accurately.
#[inline]
unsafe fn multiply_exponent_extended<F, M>(mut fp: ExtendedFloat<M>, base: u32, exponent: i32, truncated: bool)
    -> (ExtendedFloat<M>, bool)
    where M: FloatErrors,
          F: FloatRounding<M>,
          ExtendedFloat<M>: CachedPowers<M>
{
    let powers = ExtendedFloat::<M>::get_powers(base);
    let exponent = exponent + powers.bias;
    let small_index = exponent % powers.step;
    let large_index = exponent / powers.step;
    if exponent < 0 {
        // Guaranteed underflow (assign 0).
        (ExtendedFloat { frac: M::ZERO, exp: 0 }, true)
    } else if large_index as usize >= powers.large.len() {
        // Overflow (assign infinity)
        (ExtendedFloat { frac: M::ONE << 63, exp: 0x7FF }, true)
    } else {
        // Within the valid exponent range, multiply by the large and small
        // exponents and return the resulting value.

        // Track errors to as a factor of unit in last-precision.
        let mut errors: u32 = truncated as u32 * M::error_halfscale();

        // Multiply by the small power.
        // Check if we can directly multiply by an integer, if not,
        // use extended-precision multiplication.
        match fp.frac.overflowing_mul(powers.get_small_int(small_index as usize)) {
            // Overflow, multiplication unsuccessful, go slow path.
            (_, true)     => {
                fp.normalize();
                fp.imul(powers.get_small(small_index as usize));
                errors += M::error_halfscale();
            },
            // No overflow, multiplication successful.
            (frac, false) => {
                fp.frac = frac;
                fp.normalize();
            },
        }

        // Multiply by the large power
        fp.imul(powers.get_large(large_index as usize));
        errors += (errors > 0) as u32;
        errors += M::error_halfscale();

        // Normalize the floating point (and the errors).
        let shift = fp.normalize();
        errors <<= shift;

        (fp, M::error_is_accurate::<F>(errors, &fp))
    }
}

/// Create a precise native float using an intermediate extended-precision float.
///
/// Return the float approximation and if the value can be accurately
/// represented with mantissa bits of precision.
#[inline]
pub(super) fn to_extended<F, M>(mantissa: M, base: u32, exponent: i32, truncated: bool)
    -> (F, bool)
    where M: FloatErrors,
          F: FloatRounding<M>,
          ExtendedFloat<M>: CachedPowers<M>
{
    let fp = ExtendedFloat { frac: mantissa, exp: 0 };
    let (fp, valid) = unsafe { multiply_exponent_extended::<F, M>(fp, base, exponent, truncated) };
    if valid {
        (fp.as_float::<F>(), true)
    } else {
        (F::ZERO, false)
    }
}

// ATOF/ATOD

/// Parse native float from string.
///
/// The float string must be non-special, non-zero, and positive.
#[inline]
unsafe extern "C" fn to_native<F>(base: u32, first: *const u8, last: *const u8, lossy: bool)
    -> (F, *const u8)
    where F: FloatRounding<u64> + FloatRounding<u128> + StablePower
{
    let (mantissa, exponent, p, truncated) = parse_float::<u64>(base, first, last);
    let pow2_exp = pow2_exponent(base);

    if mantissa == 0 {
        // Literal 0, return early.
        // Value cannot be truncated, since we discard leading 0s whenever we
        // have mantissa == 0.
        return (F::ZERO, p);
    } else if pow2_exp != 0 {
        // We have a power of 2, can get an exact value even if the mantissa
        // was truncated, since we introduce no rounding error during
        // multiplication. If the value is **exactly halfway** and
        // truncated, we need to use the bigfloat algorithm immediately.
        if truncated && is_halfway::<F>(mantissa) {
            // Need to determine if **any** of the truncated bits is non-zero.
            // Multiplication will be super-cheap here, so this isn't actually
            // that slow.
            let (bigfloat, p) = Bigfloat::from_bytes(base, first, last);
            return (bigfloat.as_float::<F>(), p);
        } else {
            // Not truncated straddling halfway, can get exact representation.
            let float = pow2_to_exact::<F>(mantissa, base, pow2_exp, exponent);
            return (float, p);
        }
    } else if !truncated {
        // Try last fast path to exact, no mantissa truncation
        let (float, valid) = to_exact::<F>(mantissa, base, exponent);
        if valid {
            return (float, p);
        }
    }

    // Moderate path (use an extended 80-bit representation).
    let (float, valid) = to_extended::<F, _>(mantissa, base, exponent, truncated);
    if valid {
        return (float, p);
    }

    // Slow path
    if lossy {
        // Fast slow-path (use a 128-bit mantissa and extended 160-bit float).
        let (mantissa, exponent, p, truncated) = parse_float::<u128>(base, first, last);
        let (float, _) = to_extended::<F, _>(mantissa, base, exponent, truncated);
        return (float, p);
    } else {
        // Extremely slow algorithm, use arbitrary-precision float.
        let (bigfloat, p) = Bigfloat::from_bytes(base, first, last);
        return (bigfloat.as_float::<F>(), p);
    }
}

/// Parse 32-bit float from string.
#[inline]
pub(crate) unsafe extern "C" fn atof(base: u32, first: *const u8, last: *const u8)
    -> (f32, *const u8)
{
    to_native::<f32>(base, first, last, false)
}

/// Parse 64-bit float from string.
#[inline]
pub(crate) unsafe extern "C" fn atod(base: u32, first: *const u8, last: *const u8)
    -> (f64, *const u8)
{
    to_native::<f64>(base, first, last, false)
}

/// Parse 32-bit float from string.
#[inline]
pub(crate) unsafe extern "C" fn atof_lossy(base: u32, first: *const u8, last: *const u8)
    -> (f32, *const u8)
{
    to_native::<f32>(base, first, last, true)
}

/// Parse 64-bit float from string.
#[inline]
pub(crate) unsafe extern "C" fn atod_lossy(base: u32, first: *const u8, last: *const u8)
    -> (f64, *const u8)
{
    to_native::<f64>(base, first, last, true)
}

// TESTS
// -----

#[cfg(test)]
mod tests {
    use test::*;
    use super::*;

    unsafe fn check_parse_exponent(base: u32, s: &str, tup: (i32, usize)) {
        let first = s.as_ptr();
        let last = first.add(s.len());
        let (v, p) = parse_exponent(base, first, last);
        assert_eq!(v, tup.0);
        assert_eq!(distance(first, p), tup.1);
    }

    #[test]
    fn parse_exponent_test() {
        unsafe {
            // empty
            check_parse_exponent(10, "", (0, 0));

            // lowercase
            check_parse_exponent(10, "e20", (20, 3));
            check_parse_exponent(10, "e+20", (20, 4));
            check_parse_exponent(10, "e-20", (-20, 4));

            // uppercase
            check_parse_exponent(10, "E20", (20, 3));
            check_parse_exponent(10, "E+20", (20, 4));
            check_parse_exponent(10, "E-20", (-20, 4));

            // >= base15
            check_parse_exponent(15, "^20", (30, 3));
            check_parse_exponent(15, "^+20", (30, 4));
            check_parse_exponent(15, "^-20", (-30, 4));

            // overflow
            check_parse_exponent(10, "e10000000000", (i32::max_value(), 12));
            check_parse_exponent(10, "e+10000000000", (i32::max_value(), 13));
            check_parse_exponent(10, "e-10000000000", (-i32::max_value(), 13));

            // trailing
            check_parse_exponent(10, "e20 ", (20, 3));
            check_parse_exponent(10, "e+20 ", (20, 4));
        }
    }

    unsafe fn check_parse_mantissa<M>(base: u32, s: &str, tup: (M, i32, usize, bool))
        where M: Mantissa
    {
        let first = s.as_ptr();
        let last = first.add(s.len());
        let (v, d, p, t) = parse_mantissa::<M>(base, first, last);
        assert_eq!(v, tup.0);
        assert_eq!(d, tup.1);
        assert_eq!(distance(first, p), tup.2);
        assert_eq!(t, tup.3);
    }

    #[test]
    fn parse_mantissa_test() {
        unsafe {
            // 64-bit
            check_parse_mantissa::<u64>(10, "1.2345", (12345, 4, 6, false));
            check_parse_mantissa::<u64>(10, "12.345", (12345, 3, 6, false));
            check_parse_mantissa::<u64>(10, "12345.6789", (123456789, 4, 10, false));
            check_parse_mantissa::<u64>(10, "1.2345e10", (12345, 4, 6, false));
            check_parse_mantissa::<u64>(10, "0.0000000000000000001", (1, 19, 21, false));
            check_parse_mantissa::<u64>(10, "0.00000000000000000000000000001", (1, 29, 31, false));
            check_parse_mantissa::<u64>(10, "100000000000000000000", (10000000000000000000, -1, 21, true));

            // Adapted from failures in strtod.
            check_parse_mantissa::<u64>(10, "179769313486231580793728971405303415079934132710037826936173778980444968292764750946649017977587207096330286416692887910946555547851940402630657488671505820681908902000708383676273854845817711531764475730270069855571366959622842914819860834936475292719074168444365510704342711559699508093042880177904174497791.9999999999999999999999999999999999999999999999999999999999999999999999", (17976931348623158079, -289, 380, true));

            // 128-bit
            check_parse_mantissa::<u128>(10, "1.2345", (12345, 4, 6, false));
            check_parse_mantissa::<u128>(10, "12.345", (12345, 3, 6, false));
            check_parse_mantissa::<u128>(10, "12345.6789", (123456789, 4, 10, false));
            check_parse_mantissa::<u128>(10, "1.2345e10", (12345, 4, 6, false));
            check_parse_mantissa::<u128>(10, "0.0000000000000000001", (1, 19, 21, false));
            check_parse_mantissa::<u128>(10, "0.00000000000000000000000000001", (1, 29, 31, false));
            check_parse_mantissa::<u128>(10, "100000000000000000000", (100000000000000000000, 0, 21, false));
        }
    }

    #[test]
    fn normalize_mantissa_test() {
        assert_eq!(normalize_mantissa::<u64>(100, 10, 0), (1, 2));
        assert_eq!(normalize_mantissa::<u64>(101, 10, 0), (101, 0));
        assert_eq!(normalize_mantissa::<u64>(110, 10, 0), (11, 1));
    }

    unsafe fn check_parse_float<M>(base: u32, s: &str, tup: (M, i32, usize, bool))
        where M: Mantissa
    {
        let first = s.as_ptr();
        let last = first.add(s.len());
        let (v, e, p, t) = parse_float::<M>(base, first, last);
        assert_eq!(v, tup.0);
        assert_eq!(e, tup.1);
        assert_eq!(distance(first, p), tup.2);
        assert_eq!(t, tup.3);
    }

    #[test]
    fn parse_float_test() {
        unsafe {
            // 64-bit
            check_parse_float::<u64>(10, "1.2345", (12345, -4, 6, false));
            check_parse_float::<u64>(10, "12.345", (12345, -3, 6, false));
            check_parse_float::<u64>(10, "12345.6789", (123456789, -4, 10, false));
            check_parse_float::<u64>(10, "1.2345e10", (12345, 6, 9, false));
            check_parse_float::<u64>(10, "100000000000000000000", (1, 20, 21, true));
            check_parse_float::<u64>(10, "100000000000000000001", (1, 20, 21, true));

            // Adapted from failures in strtod.
            check_parse_float::<u64>(10, "179769313486231580793728971405303415079934132710037826936173778980444968292764750946649017977587207096330286416692887910946555547851940402630657488671505820681908902000708383676273854845817711531764475730270069855571366959622842914819860834936475292719074168444365510704342711559699508093042880177904174497791.9999999999999999999999999999999999999999999999999999999999999999999999", (17976931348623158079, 289, 380, true));

            // 128-bit
            check_parse_float::<u128>(10, "1.2345", (12345, -4, 6, false));
            check_parse_float::<u128>(10, "12.345", (12345, -3, 6, false));
            check_parse_float::<u128>(10, "12345.6789", (123456789, -4, 10, false));
            check_parse_float::<u128>(10, "1.2345e10", (12345, 6, 9, false));
            check_parse_float::<u128>(10, "100000000000000000000", (1, 20, 21, false));
            check_parse_float::<u128>(10, "100000000000000000001", (100000000000000000001, 0, 21, false));
        }
    }

    #[test]
    fn is_halfway_test() {
        // Variant of b1000000000000000000000001, a halfway value for f32.
        assert!(is_halfway::<f32>(0x1000001));
        assert!(is_halfway::<f32>(0x2000002));
        assert!(is_halfway::<f32>(0x8000008000000000));
        assert!(!is_halfway::<f64>(0x1000001));
        assert!(!is_halfway::<f64>(0x2000002));
        assert!(!is_halfway::<f64>(0x8000008000000000));

        // Variant of b10000000000000000000000001, which is 1-off a halfway value.
        assert!(!is_halfway::<f32>(0x2000001));
        assert!(!is_halfway::<f64>(0x2000001));

        // Variant of b100000000000000000000000000000000000000000000000000001,
        // a halfway value for f64
        assert!(!is_halfway::<f32>(0x20000000000001));
        assert!(!is_halfway::<f32>(0x40000000000002));
        assert!(!is_halfway::<f32>(0x8000000000000400));
        assert!(is_halfway::<f64>(0x20000000000001));
        assert!(is_halfway::<f64>(0x40000000000002));
        assert!(is_halfway::<f64>(0x8000000000000400));

        // Variant of b111111000000000000000000000000000000000000000000000001,
        // a halfway value for f64.
        assert!(!is_halfway::<f32>(0x3f000000000001));
        assert!(!is_halfway::<f32>(0xFC00000000000400));
        assert!(is_halfway::<f64>(0x3f000000000001));
        assert!(is_halfway::<f64>(0xFC00000000000400));

        // Variant of b1000000000000000000000000000000000000000000000000000001,
        // which is 1-off a halfway value.
        assert!(!is_halfway::<f32>(0x40000000000001));
        assert!(!is_halfway::<f64>(0x40000000000001));
    }

    #[test]
    fn pow2_to_float_exact_test() {
        // Everything is valid.
        let mantissa = 1 << 63;
        for base in BASE_POW2.iter().cloned() {
            let (min_exp, max_exp) = f32::exponent_limit(base);
            let pow2_exp = pow2_exponent(base);
            for exp in min_exp-20..max_exp+30 {
                // Always valid, ignore result
                pow2_to_exact::<f32>(mantissa, base, pow2_exp, exp);
            }
        }
    }

    #[test]
    fn pow2_to_double_exact_test() {
        // Everything is valid.
        let mantissa = 1 << 63;
        for base in BASE_POW2.iter().cloned() {
            let (min_exp, max_exp) = f64::exponent_limit(base);
            let pow2_exp = pow2_exponent(base);
            for exp in min_exp-20..max_exp+30 {
                // Ignore result, always valid
                pow2_to_exact::<f64>(mantissa, base, pow2_exp, exp);
            }
        }
    }

    #[test]
    fn to_float_exact_test() {
        // valid
        let mantissa = 1 << (f32::MANTISSA_SIZE - 1);
        for base in BASE_POWN.iter().cloned() {
            let (min_exp, max_exp) = f32::exponent_limit(base);
            for exp in min_exp..max_exp+1 {
                let (_, valid) = to_exact::<f32>(mantissa, base, exp);
                assert!(valid, "should be valid {:?}.", (mantissa, base, exp));
            }
        }

        // invalid mantissa
        let (_, valid) = to_exact::<f32>(1<<f32::MANTISSA_SIZE, 3, 0);
        assert!(!valid, "invalid mantissa");

        // invalid exponents
        for base in BASE_POWN.iter().cloned() {
            let (min_exp, max_exp) = f32::exponent_limit(base);
            let (_, valid) = to_exact::<f32>(mantissa, base, min_exp-1);
            assert!(!valid, "exponent under min_exp");

            let (_, valid) = to_exact::<f32>(mantissa, base, max_exp+1);
            assert!(!valid, "exponent above max_exp");
        }
    }

    #[test]
    fn to_double_exact_test() {
        // valid
        let mantissa = 1 << (f64::MANTISSA_SIZE - 1);
        for base in BASE_POWN.iter().cloned() {
            let (min_exp, max_exp) = f64::exponent_limit(base);
            for exp in min_exp..max_exp+1 {
                let (_, valid) = to_exact::<f64>(mantissa, base, exp);
                assert!(valid, "should be valid {:?}.", (mantissa, base, exp));
            }
        }

        // invalid mantissa
        let (_, valid) = to_exact::<f64>(1<<f64::MANTISSA_SIZE, 3, 0);
        assert!(!valid, "invalid mantissa");

        // invalid exponents
        for base in BASE_POWN.iter().cloned() {
            let (min_exp, max_exp) = f64::exponent_limit(base);
            let (_, valid) = to_exact::<f64>(mantissa, base, min_exp-1);
            assert!(!valid, "exponent under min_exp");

            let (_, valid) = to_exact::<f64>(mantissa, base, max_exp+1);
            assert!(!valid, "exponent above max_exp");
        }
    }

    #[test]
    fn to_float_extended_test() {
        // valid (overflowing small mult)
        let mantissa: u64 = 1 << 63;
        let (f, valid) = to_extended::<f32, _>(mantissa, 3, 1, false);
        assert_eq!(f, 2.7670116e+19);
        assert!(valid, "exponent should be valid");

        let mantissa: u64 = 4746067219335938;
        let (f, valid) = to_extended::<f32, _>(mantissa, 15, -9, false);
        assert_eq!(f, 123456.1);
        assert!(valid, "exponent should be valid");
    }

    #[test]
    fn to_double_extended_test() {
        // valid (overflowing small mult)
        let mantissa: u64 = 1 << 63;
        let (f, valid) = to_extended::<f64, _>(mantissa, 3, 1, false);
        assert_eq!(f, 2.7670116110564327e+19);
        assert!(valid, "exponent should be valid");

        // valid (ends of the earth, salting the earth)
        let (f, valid) = to_extended::<f64, _>(mantissa, 3, -695, true);
        assert_eq!(f, 2.32069302345e-313);
        assert!(valid, "exponent should be valid");

        // invalid ("268A6.177777778", base 15)
        let mantissa: u64 = 4746067219335938;
        let (_, valid) = to_extended::<f64, _>(mantissa, 15, -9, false);
        assert!(!valid, "exponent should be invalid");

        // valid ("268A6.177777778", base 15)
        // 123456.10000000001300614743687445, exactly, should not round up.
        let mantissa: u128 = 4746067219335938;
        let (f, valid) = to_extended::<f64, _>(mantissa, 15, -9, false);
        assert_eq!(f, 123456.1);
        assert!(valid, "exponent should be valid");
    }

    unsafe fn check_atof(base: u32, s: &str, tup: (f32, usize)) {
        let first = s.as_ptr();
        let last = first.add(s.len());
        let (v, p) = atof(base, first, last);
        assert_f32_eq!(v, tup.0);
        assert_eq!(distance(first, p), tup.1);
    }

    #[test]
    fn atof_test() {
        unsafe {
            check_atof(10, "1.2345", (1.2345, 6));
            check_atof(10, "12.345", (12.345, 6));
            check_atof(10, "12345.6789", (12345.6789, 10));
            check_atof(10, "1.2345e10", (1.2345e10, 9));
            check_atof(10, "1.2345e-38", (1.2345e-38, 10));

            // Check expected rounding, using borderline cases.
            // Round-down, halfway
            check_atof(10, "16777216", (16777216.0, 8));
            check_atof(10, "16777217", (16777216.0, 8));
            check_atof(10, "16777218", (16777218.0, 8));
            check_atof(10, "33554432", (33554432.0, 8));
            check_atof(10, "33554434", (33554432.0, 8));
            check_atof(10, "33554436", (33554436.0, 8));
            check_atof(10, "17179869184", (17179869184.0, 11));
            check_atof(10, "17179870208", (17179869184.0, 11));
            check_atof(10, "17179871232", (17179871232.0, 11));

            // Round-up, halfway
            check_atof(10, "16777218", (16777218.0, 8));
            check_atof(10, "16777219", (16777220.0, 8));
            check_atof(10, "16777220", (16777220.0, 8));
            check_atof(10, "33554436", (33554436.0, 8));
            check_atof(10, "33554438", (33554440.0, 8));
            check_atof(10, "33554440", (33554440.0, 8));
            check_atof(10, "17179871232", (17179871232.0, 11));
            check_atof(10, "17179872256", (17179873280.0, 11));
            check_atof(10, "17179873280", (17179873280.0, 11));

            // Round-up, above halfway
            check_atof(10, "33554435", (33554436.0, 8));
            check_atof(10, "17179870209", (17179871232.0, 11));
        }
    }

    unsafe fn check_atod(base: u32, s: &str, tup: (f64, usize)) {
        let first = s.as_ptr();
        let last = first.add(s.len());
        let (v, p) = atod(base, first, last);
        assert_f64_eq!(v, tup.0);
        assert_eq!(distance(first, p), tup.1);
    }

    #[test]
    fn atod_test() {
        unsafe {
            check_atod(10, "1.2345", (1.2345, 6));
            check_atod(10, "12.345", (12.345, 6));
            check_atod(10, "12345.6789", (12345.6789, 10));
            check_atod(10, "1.2345e10", (1.2345e10, 9));
            check_atod(10, "1.2345e-308", (1.2345e-308, 11));

            // Check expected rounding, using borderline cases.
            // Round-down, halfway
            check_atod(10, "9007199254740992", (9007199254740992.0, 16));
            check_atod(10, "9007199254740993", (9007199254740992.0, 16));
            check_atod(10, "9007199254740994", (9007199254740994.0, 16));
            check_atod(10, "18014398509481984", (18014398509481984.0, 17));
            check_atod(10, "18014398509481986", (18014398509481984.0, 17));
            check_atod(10, "18014398509481988", (18014398509481988.0, 17));
            check_atod(10, "9223372036854775808", (9223372036854775808.0, 19));
            check_atod(10, "9223372036854776832", (9223372036854775808.0, 19));
            check_atod(10, "9223372036854777856", (9223372036854777856.0, 19));
            check_atod(10, "11417981541647679048466287755595961091061972992", (11417981541647679048466287755595961091061972992.0, 47));
            check_atod(10, "11417981541647680316116887983825362587765178368", (11417981541647679048466287755595961091061972992.0, 47));
            check_atod(10, "11417981541647681583767488212054764084468383744", (11417981541647681583767488212054764084468383744.0, 47));

            // Round-up, halfway
            check_atod(10, "9007199254740994", (9007199254740994.0, 16));
            check_atod(10, "9007199254740995", (9007199254740996.0, 16));
            check_atod(10, "9007199254740996", (9007199254740996.0, 16));
            check_atod(10, "18014398509481988", (18014398509481988.0, 17));
            check_atod(10, "18014398509481990", (18014398509481992.0, 17));
            check_atod(10, "18014398509481992", (18014398509481992.0, 17));
            check_atod(10, "9223372036854777856", (9223372036854777856.0, 19));
            check_atod(10, "9223372036854778880", (9223372036854779904.0, 19));
            check_atod(10, "9223372036854779904", (9223372036854779904.0, 19));
            check_atod(10, "11417981541647681583767488212054764084468383744", (11417981541647681583767488212054764084468383744.0, 47));
            check_atod(10, "11417981541647682851418088440284165581171589120", (11417981541647684119068688668513567077874794496.0, 47));
            check_atod(10, "11417981541647684119068688668513567077874794496", (11417981541647684119068688668513567077874794496.0, 47));

            // Round-up, above halfway
            check_atod(10, "9223372036854776833", (9223372036854777856.0, 19));
            check_atod(10, "11417981541647680316116887983825362587765178369", (11417981541647681583767488212054764084468383744.0, 47));

            // Rounding error
            // Adapted from failures in strtod.
            check_atod(10, "2.2250738585072014e-308", (2.2250738585072014e-308, 23));
            check_atod(10, "2.22507385850720113605740979670913197593481954635164564802342610972482222202107694551652952390813508791414915891303962110687008643869459464552765720740782062174337998814106326732925355228688137214901298112245145188984905722230728525513315575501591439747639798341180199932396254828901710708185069063066665599493827577257201576306269066333264756530000924588831643303777979186961204949739037782970490505108060994073026293712895895000358379996720725430436028407889577179615094551674824347103070260914462157228988025818254518032570701886087211312807951223342628836862232150377566662250398253433597456888442390026549819838548794829220689472168983109969836584681402285424333066033985088644580400103493397042756718644338377048603786162277173854562306587467901408672332763671875e-308", (2.2250738585072014e-308, 774));
        }
    }

    // Lossy

    unsafe fn check_atof_lossy(base: u32, s: &str, tup: (f32, usize)) {
        let first = s.as_ptr();
        let last = first.add(s.len());
        let (v, p) = atof_lossy(base, first, last);
        assert_f32_eq!(v, tup.0);
        assert_eq!(distance(first, p), tup.1);
    }

    #[test]
    fn atof_lossy_test() {
        unsafe {
            check_atof_lossy(10, "1.2345", (1.2345, 6));
            check_atof_lossy(10, "12.345", (12.345, 6));
            check_atof_lossy(10, "12345.6789", (12345.6789, 10));
            check_atof_lossy(10, "1.2345e10", (1.2345e10, 9));
        }
    }

    unsafe fn check_atod_lossy(base: u32, s: &str, tup: (f64, usize)) {
        let first = s.as_ptr();
        let last = first.add(s.len());
        let (v, p) = atod_lossy(base, first, last);
        assert_f64_eq!(v, tup.0);
        assert_eq!(distance(first, p), tup.1);
    }

    #[test]
    fn atod_lossy_test() {
        unsafe {
            check_atod_lossy(10, "1.2345", (1.2345, 6));
            check_atod_lossy(10, "12.345", (12.345, 6));
            check_atod_lossy(10, "12345.6789", (12345.6789, 10));
            check_atod_lossy(10, "1.2345e10", (1.2345e10, 9));
        }
    }
}
