#![cfg(feature = "radix")]

mod stackvec;

use lexical_parse_float::bigint::{Bigfloat, LIMB_BITS};
use lexical_parse_float::float::ExtendedFloat80;
use stackvec::vec_from_u32;

#[test]
fn simple_test() {
    let x = Bigfloat::new();
    assert_eq!(x.exp, 0);

    let y = Bigfloat::from_float(ExtendedFloat80 {
        mant: 1 << 63,
        exp: -63,
    });
    assert_eq!(y.exp, -63);

    let x = Bigfloat::from_u32(1);
    assert_eq!(&*x.data, &[1]);

    let mut x = Bigfloat::from_u64(1);
    assert_eq!(&*x.data, &[1]);

    x.pow(10, 10);
    assert_eq!(&*x.data, &[9765625]);
    assert_eq!(x.exp, 10);

    x.shl_bits(1);
    assert_eq!(&*x.data, &[19531250]);
    assert_eq!(x.exp, 10);

    x.shl_limbs(1);
    assert_eq!(&*x.data, &[19531250]);
    assert_eq!(x.exp, 11);

    assert_eq!(x.leading_zeros(), LIMB_BITS as u32 - 25);

    x *= &y;
    let expected = vec_from_u32(&[0, 0, 9765625]);
    assert!(x.data == expected, "failed");
    assert_eq!(x.exp, -52);
}

#[test]
fn leading_zeros_test() {
    assert_eq!(Bigfloat::new().leading_zeros(), 0);

    assert_eq!(Bigfloat::from_u32(0xFF).leading_zeros(), LIMB_BITS as u32 - 8);
    assert_eq!(Bigfloat::from_u64(0xFF00000000).leading_zeros(), 24);

    assert_eq!(Bigfloat::from_u32(0xF).leading_zeros(), LIMB_BITS as u32 - 4);
    assert_eq!(Bigfloat::from_u64(0xF00000000).leading_zeros(), 28);

    assert_eq!(Bigfloat::from_u32(0xF0).leading_zeros(), LIMB_BITS as u32 - 8);
    assert_eq!(Bigfloat::from_u64(0xF000000000).leading_zeros(), 24);
}
