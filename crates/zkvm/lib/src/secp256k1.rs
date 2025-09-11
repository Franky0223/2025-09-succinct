use crate::{
    syscall_secp256k1_add, syscall_secp256k1_double,
    utils::{AffinePoint, WeierstrassAffinePoint, WeierstrassPoint},
};

/// The number of limbs in [Secp256k1Point].
pub const N: usize = 8;

/// An affine point on the Secp256k1 curve.
#[derive(Copy, Clone, Debug)]
#[repr(align(8))]
pub struct Secp256k1Point(pub WeierstrassPoint<N>);

impl WeierstrassAffinePoint<N> for Secp256k1Point {
    fn infinity() -> Self {
        Self(WeierstrassPoint::Infinity)
    }

    fn is_infinity(&self) -> bool {
        matches!(self.0, WeierstrassPoint::Infinity)
    }
}

impl AffinePoint<N> for Secp256k1Point {
    /// The values are taken from https://en.bitcoin.it/wiki/Secp256k1.
    const GENERATOR: [u64; N] = [
        6481385041966929816,
        188021827762530521,
        6170039885052185351,
        8772561819708210092,
        11261198710074299576,
        18237243440184513561,
        6747795201694173352,
        5204712524664259685,
    ];

    #[allow(deprecated)]
    const GENERATOR_T: Self = Self(WeierstrassPoint::Affine(Self::GENERATOR));

    fn new(limbs: [u64; N]) -> Self {
        Self(WeierstrassPoint::Affine(limbs))
    }

    fn identity() -> Self {
        Self::infinity()
    }

    fn is_identity(&self) -> bool {
        self.is_infinity()
    }

    fn limbs_ref(&self) -> &[u64; N] {
        match &self.0 {
            WeierstrassPoint::Infinity => panic!("Infinity point has no limbs"),
            WeierstrassPoint::Affine(limbs) => limbs,
        }
    }

    fn limbs_mut(&mut self) -> &mut [u64; N] {
        match &mut self.0 {
            WeierstrassPoint::Infinity => panic!("Infinity point has no limbs"),
            WeierstrassPoint::Affine(limbs) => limbs,
        }
    }

    fn add_assign(&mut self, other: &Self) {
        let a = self.limbs_mut();
        let b = other.limbs_ref();
        unsafe {
            syscall_secp256k1_add(a, b);
        }
    }

    fn complete_add_assign(&mut self, other: &Self) {
        self.weierstrass_add_assign(other);
    }

    fn double(&mut self) {
        match &mut self.0 {
            WeierstrassPoint::Infinity => (),
            WeierstrassPoint::Affine(limbs) => unsafe {
                syscall_secp256k1_double(limbs);
            },
        }
    }
}
