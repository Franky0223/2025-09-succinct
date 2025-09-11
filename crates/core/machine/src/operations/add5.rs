use slop_algebra::{AbstractField, Field};
use sp1_derive::AlignedBorrow;

use sp1_core_executor::events::ByteRecord;
use sp1_hypercube::air::SP1AirBuilder;
use sp1_primitives::consts::u32_to_u16_limbs;

use crate::{air::WordAirBuilder, utils::u32_to_half_word};

/// A set of columns needed to compute the sum of five u32s as u32.
#[derive(AlignedBorrow, Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct Add5Operation<T> {
    /// The result of `a + b + c + d + e`.
    pub value: [T; 2],
}

impl<F: Field> Add5Operation<F> {
    #[allow(clippy::too_many_arguments)]
    pub fn populate(
        &mut self,
        record: &mut impl ByteRecord,
        a_u32: u32,
        b_u32: u32,
        c_u32: u32,
        d_u32: u32,
        e_u32: u32,
    ) -> u32 {
        let expected =
            a_u32.wrapping_add(b_u32).wrapping_add(c_u32).wrapping_add(d_u32).wrapping_add(e_u32);
        let expected_limbs = u32_to_u16_limbs(expected);
        self.value = u32_to_half_word(expected);
        let a = u32_to_u16_limbs(a_u32);
        let b = u32_to_u16_limbs(b_u32);
        let c = u32_to_u16_limbs(c_u32);
        let d = u32_to_u16_limbs(d_u32);
        let e = u32_to_u16_limbs(e_u32);
        let base = 1u32 << 16;
        let mut carry = 0;
        let mut carry_limbs = [0u8; 2];
        for i in 0..2 {
            carry = ((a[i] as u32)
                + (b[i] as u32)
                + (c[i] as u32)
                + (d[i] as u32)
                + (e[i] as u32)
                + carry
                - expected_limbs[i] as u32)
                / base;
            carry_limbs[i] = carry as u8;
        }

        // Range check.
        record.add_u16_range_checks(&expected_limbs);
        record.add_u8_range_checks(&carry_limbs);
        expected
    }

    /// Evaluate the add5 operation.
    /// Assumes that the five words are valid u32s of two u16 limbs.
    /// Constrains that `is_real` is boolean.
    /// If `is_real` is true, the `value` is constrained to a valid u32 representing the sum.
    pub fn eval<AB: SP1AirBuilder>(
        builder: &mut AB,
        words: &[[AB::Expr; 2]; 5],
        is_real: AB::Var,
        cols: Add5Operation<AB::Var>,
    ) {
        builder.assert_bool(is_real);

        let base = AB::F::from_canonical_u32(1 << 16);
        let mut carry_limbs = [AB::Expr::zero(), AB::Expr::zero()];
        let mut carry = AB::Expr::zero(); // Initialize carry to zero

        // The set of constraints are
        //  - carry is initialized to zero
        //  - 2^16 * carry_next + value[i] = sum(word[i]) + carry
        //  - 0 <= carry < 2^8
        //  - 0 <= value[i] < 2^16
        // Since the carries are bounded by 2^8, no SP1Field overflows are possible.
        // The maximum carry possible is less than 2^8, so the circuit is complete.
        for i in 0..2 {
            carry = (words[0][i].clone()
                + words[1][i].clone()
                + words[2][i].clone()
                + words[3][i].clone()
                + words[4][i].clone()
                - cols.value[i]
                + carry.clone())
                * base.inverse();
            carry_limbs[i] = carry.clone();
        }
        // Range check each limb.
        builder.slice_range_check_u16(&cols.value, is_real);
        builder.slice_range_check_u8(&carry_limbs, is_real);
    }
}
