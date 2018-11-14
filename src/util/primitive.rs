//! Utilities for Rust primitives.

use lib::fmt;
use super::cast::AsCast;

/// Type that can be converted to primitive with `as`.
pub trait AsPrimitive: Copy + PartialEq + PartialOrd {
    fn as_u8(self) -> u8;
    fn as_u16(self) -> u16;
    fn as_u32(self) -> u32;
    fn as_u64(self) -> u64;
    fn as_usize(self) -> usize;
    fn as_i8(self) -> i8;
    fn as_i16(self) -> i16;
    fn as_i32(self) -> i32;
    fn as_i64(self) -> i64;
    fn as_isize(self) -> isize;
    fn as_f32(self) -> f32;
    fn as_f64(self) -> f64;
}

macro_rules! as_primitive {
    ($($t:ty)*) => ($(
        impl AsPrimitive for $t {
            #[inline(always)]
            fn as_u8(self) -> u8 { self as u8 }

            #[inline(always)]
            fn as_u16(self) -> u16 { self as u16 }

            #[inline(always)]
            fn as_u32(self) -> u32 { self as u32 }

            #[inline(always)]
            fn as_u64(self) -> u64 { self as u64 }

            #[inline(always)]
            fn as_usize(self) -> usize { self as usize }

            #[inline(always)]
            fn as_i8(self) -> i8 { self as i8 }

            #[inline(always)]
            fn as_i16(self) -> i16 { self as i16 }

            #[inline(always)]
            fn as_i32(self) -> i32 { self as i32 }

            #[inline(always)]
            fn as_i64(self) -> i64 { self as i64 }

            #[inline(always)]
            fn as_isize(self) -> isize { self as isize }

            #[inline(always)]
            fn as_f32(self) -> f32 { self as f32 }

            #[inline(always)]
            fn as_f64(self) -> f64 { self as f64 }
        }
    )*)
}

as_primitive! { u8 u16 u32 u64 usize i8 i16 i32 i64 isize f32 f64 }

// PRIMITIVE

/// Primitive type trait.
pub trait Primitive: AsCast + fmt::Debug + fmt::Display {
}

macro_rules! primitive {
    ($($t:ty)*) => ($(
        impl Primitive for $t {}
    )*)
}

primitive! { u8 u16 u32 u64 usize i8 i16 i32 i64 isize f32 f64 }

// TEST
// ----

#[cfg(test)]
mod tests {
    use super::*;

    fn check_as_primitive<T: AsPrimitive>(t: T) {
        let _: u8 = t.as_u8();
        let _: u16 = t.as_u16();
        let _: u32 = t.as_u32();
        let _: u64 = t.as_u64();
        let _: usize = t.as_usize();
        let _: i8 = t.as_i8();
        let _: i16 = t.as_i16();
        let _: i32 = t.as_i32();
        let _: i64 = t.as_i64();
        let _: isize = t.as_isize();
        let _: f32 = t.as_f32();
        let _: f64 = t.as_f64();
    }

    #[test]
    fn as_primitive_test() {
        check_as_primitive(1u8);
        check_as_primitive(1u16);
        check_as_primitive(1u32);
        check_as_primitive(1u64);
        check_as_primitive(1usize);
        check_as_primitive(1i8);
        check_as_primitive(1i16);
        check_as_primitive(1i32);
        check_as_primitive(1i64);
        check_as_primitive(1isize);
        check_as_primitive(1f32);
        check_as_primitive(1f64);
    }
}
