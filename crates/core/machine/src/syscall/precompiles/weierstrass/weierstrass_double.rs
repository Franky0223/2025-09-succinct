use crate::{
    air::SP1CoreAirBuilder,
    memory::MemoryAccessColsU8,
    operations::{
        field::{field_op::FieldOpCols, range::FieldLtCols},
        AddrAddOperation, AddressSlicePageProtOperation, SyscallAddrOperation,
    },
    utils::{limbs_to_words, next_multiple_of_32, zeroed_f_vec},
};
use core::{
    borrow::{Borrow, BorrowMut},
    mem::size_of,
};
use generic_array::GenericArray;
use itertools::Itertools;
use num::{BigUint, One, Zero};
use slop_air::{Air, BaseAir};
use slop_algebra::{AbstractField, PrimeField32};
use slop_matrix::{dense::RowMajorMatrix, Matrix};
use slop_maybe_rayon::prelude::{ParallelBridge, ParallelIterator, ParallelSlice};
use sp1_core_executor::{
    events::{
        ByteLookupEvent, ByteRecord, EllipticCurveDoubleEvent, FieldOperation, MemoryRecordEnum,
        MemoryWriteRecord, PrecompileEvent, SyscallEvent,
    },
    syscalls::SyscallCode,
    ExecutionRecord, Program,
};
use sp1_curves::{
    params::{FieldParameters, Limbs, NumLimbs, NumWords},
    weierstrass::WeierstrassParameters,
    AffinePoint, CurveType, EllipticCurve,
};
use sp1_derive::AlignedBorrow;
use sp1_hypercube::{
    air::{InteractionScope, MachineAir},
    Word,
};
use sp1_primitives::consts::{PROT_READ, PROT_WRITE};
use std::{fmt::Debug, marker::PhantomData};

pub const fn num_weierstrass_double_cols<P: FieldParameters + NumWords>() -> usize {
    size_of::<WeierstrassDoubleAssignCols<u8, P>>()
}

/// A set of columns to double a point on a Weierstrass curve.
///
/// Right now the number of limbs is assumed to be a constant, although this could be macro-ed or
/// made generic in the future.
#[derive(Debug, Clone, AlignedBorrow)]
#[repr(C)]
pub struct WeierstrassDoubleAssignCols<T, P: FieldParameters + NumWords> {
    pub is_real: T,
    pub clk_high: T,
    pub clk_low: T,
    pub p_ptr: SyscallAddrOperation<T>,
    pub p_addrs: GenericArray<AddrAddOperation<T>, P::WordsCurvePoint>,
    pub p_access: GenericArray<MemoryAccessColsU8<T>, P::WordsCurvePoint>,
    pub slope_denominator: FieldOpCols<T, P>,
    pub slope_numerator: FieldOpCols<T, P>,
    pub slope: FieldOpCols<T, P>,
    pub p_x_squared: FieldOpCols<T, P>,
    pub p_x_squared_times_3: FieldOpCols<T, P>,
    pub slope_squared: FieldOpCols<T, P>,
    pub p_x_plus_p_x: FieldOpCols<T, P>,
    pub x3_ins: FieldOpCols<T, P>,
    pub p_x_minus_x: FieldOpCols<T, P>,
    pub y3_ins: FieldOpCols<T, P>,
    pub slope_times_p_x_minus_x: FieldOpCols<T, P>,
    pub x3_range: FieldLtCols<T, P>,
    pub y3_range: FieldLtCols<T, P>,
    pub write_slice_page_prot_access: AddressSlicePageProtOperation<T>,
}

#[derive(Default)]
pub struct WeierstrassDoubleAssignChip<E> {
    _marker: PhantomData<E>,
}

impl<E: EllipticCurve + WeierstrassParameters> WeierstrassDoubleAssignChip<E> {
    pub const fn new() -> Self {
        Self { _marker: PhantomData }
    }

    fn populate_field_ops<F: PrimeField32>(
        blu_events: &mut Vec<ByteLookupEvent>,
        cols: &mut WeierstrassDoubleAssignCols<F, E::BaseField>,
        p_x: BigUint,
        p_y: BigUint,
    ) {
        // This populates necessary field operations to double a point on a Weierstrass curve.

        let a = E::a_int();
        let slope = {
            // slope_numerator = a + (p.x * p.x) * 3.
            let slope_numerator = {
                let p_x_squared =
                    cols.p_x_squared.populate(blu_events, &p_x, &p_x, FieldOperation::Mul);
                let p_x_squared_times_3 = cols.p_x_squared_times_3.populate(
                    blu_events,
                    &p_x_squared,
                    &BigUint::from(3u32),
                    FieldOperation::Mul,
                );
                cols.slope_numerator.populate(
                    blu_events,
                    &a,
                    &p_x_squared_times_3,
                    FieldOperation::Add,
                )
            };

            // slope_denominator = 2 * y.
            let slope_denominator = cols.slope_denominator.populate(
                blu_events,
                &BigUint::from(2u32),
                &p_y,
                FieldOperation::Mul,
            );

            cols.slope.populate(
                blu_events,
                &slope_numerator,
                &slope_denominator,
                FieldOperation::Div,
            )
        };

        // x = slope * slope - (p.x + p.x).
        let x = {
            let slope_squared =
                cols.slope_squared.populate(blu_events, &slope, &slope, FieldOperation::Mul);
            let p_x_plus_p_x =
                cols.p_x_plus_p_x.populate(blu_events, &p_x, &p_x, FieldOperation::Add);
            let x3 = cols.x3_ins.populate(
                blu_events,
                &slope_squared,
                &p_x_plus_p_x,
                FieldOperation::Sub,
            );
            cols.x3_range.populate(blu_events, &x3, &E::BaseField::modulus());
            x3
        };

        // y = slope * (p.x - x) - p.y.
        {
            let p_x_minus_x = cols.p_x_minus_x.populate(blu_events, &p_x, &x, FieldOperation::Sub);
            let slope_times_p_x_minus_x = cols.slope_times_p_x_minus_x.populate(
                blu_events,
                &slope,
                &p_x_minus_x,
                FieldOperation::Mul,
            );
            let y3 = cols.y3_ins.populate(
                blu_events,
                &slope_times_p_x_minus_x,
                &p_y,
                FieldOperation::Sub,
            );
            cols.y3_range.populate(blu_events, &y3, &E::BaseField::modulus());
        }
    }
}

impl<F: PrimeField32, E: EllipticCurve + WeierstrassParameters> MachineAir<F>
    for WeierstrassDoubleAssignChip<E>
{
    type Record = ExecutionRecord;
    type Program = Program;

    fn name(&self) -> String {
        match E::CURVE_TYPE {
            CurveType::Secp256k1 => "Secp256k1DoubleAssign".to_string(),
            CurveType::Secp256r1 => "Secp256r1DoubleAssign".to_string(),
            CurveType::Bn254 => "Bn254DoubleAssign".to_string(),
            CurveType::Bls12381 => "Bls12381DoubleAssign".to_string(),
            _ => panic!("Unsupported curve"),
        }
    }

    fn num_rows(&self, input: &Self::Record) -> Option<usize> {
        let nb_rows = match E::CURVE_TYPE {
            CurveType::Secp256k1 => {
                input.get_precompile_events(SyscallCode::SECP256K1_DOUBLE).len()
            }
            CurveType::Secp256r1 => {
                input.get_precompile_events(SyscallCode::SECP256R1_DOUBLE).len()
            }
            CurveType::Bn254 => input.get_precompile_events(SyscallCode::BN254_DOUBLE).len(),
            CurveType::Bls12381 => input.get_precompile_events(SyscallCode::BLS12381_DOUBLE).len(),
            _ => panic!("Unsupported curve"),
        };
        let size_log2 = input.fixed_log2_rows::<F, _>(self);
        let padded_nb_rows = next_multiple_of_32(nb_rows, size_log2);
        Some(padded_nb_rows)
    }

    fn generate_dependencies(&self, input: &Self::Record, output: &mut Self::Record) {
        let events = match E::CURVE_TYPE {
            CurveType::Secp256k1 => &input.get_precompile_events(SyscallCode::SECP256K1_DOUBLE),
            CurveType::Secp256r1 => &input.get_precompile_events(SyscallCode::SECP256R1_DOUBLE),
            CurveType::Bn254 => &input.get_precompile_events(SyscallCode::BN254_DOUBLE),
            CurveType::Bls12381 => &input.get_precompile_events(SyscallCode::BLS12381_DOUBLE),
            _ => panic!("Unsupported curve"),
        };

        let num_cols = num_weierstrass_double_cols::<E::BaseField>();
        let chunk_size = std::cmp::max(events.len() / num_cpus::get(), 1);

        let blu_events: Vec<Vec<ByteLookupEvent>> = events
            .par_chunks(chunk_size)
            .map(|ops: &[(SyscallEvent, PrecompileEvent)]| {
                // The blu map stores shard -> map(byte lookup event -> multiplicity).
                let mut blu = Vec::new();
                ops.iter().for_each(|(_, op)| match op {
                    PrecompileEvent::Secp256k1Double(event)
                    | PrecompileEvent::Secp256r1Double(event)
                    | PrecompileEvent::Bn254Double(event)
                    | PrecompileEvent::Bls12381Double(event) => {
                        let mut row = zeroed_f_vec(num_cols);
                        let cols: &mut WeierstrassDoubleAssignCols<F, E::BaseField> =
                            row.as_mut_slice().borrow_mut();
                        Self::populate_row(
                            event,
                            cols,
                            &mut blu,
                            input.public_values.is_untrusted_programs_enabled,
                        );
                    }
                    _ => unreachable!(),
                });
                blu
            })
            .collect();

        for blu in blu_events {
            output.add_byte_lookup_events(blu);
        }
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        _: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        // collects the events based on the curve type.
        let events = match E::CURVE_TYPE {
            CurveType::Secp256k1 => input.get_precompile_events(SyscallCode::SECP256K1_DOUBLE),
            CurveType::Secp256r1 => input.get_precompile_events(SyscallCode::SECP256R1_DOUBLE),
            CurveType::Bn254 => input.get_precompile_events(SyscallCode::BN254_DOUBLE),
            CurveType::Bls12381 => input.get_precompile_events(SyscallCode::BLS12381_DOUBLE),
            _ => panic!("Unsupported curve"),
        };

        let num_cols = num_weierstrass_double_cols::<E::BaseField>();
        let num_rows = input
            .fixed_log2_rows::<F, _>(self)
            .map(|x| 1 << x)
            .unwrap_or(std::cmp::max(events.len().next_multiple_of(32), 4));
        let mut values = zeroed_f_vec(num_rows * num_cols);
        let chunk_size = 64;

        let num_words_field_element = E::BaseField::NB_LIMBS / 8;
        let mut dummy_row = zeroed_f_vec(num_cols);
        let cols: &mut WeierstrassDoubleAssignCols<F, E::BaseField> =
            dummy_row.as_mut_slice().borrow_mut();
        let dummy_memory_record = MemoryWriteRecord {
            value: 1,
            timestamp: 1,
            prev_value: 1,
            prev_timestamp: 0,
            prev_page_prot_record: None,
        };
        let zero = BigUint::zero();
        let one = BigUint::one();
        let dummy_record_enum = MemoryRecordEnum::Write(dummy_memory_record);
        cols.p_access[num_words_field_element].populate(dummy_record_enum, &mut vec![]);
        Self::populate_field_ops(&mut vec![], cols, zero, one);

        values.chunks_mut(chunk_size * num_cols).enumerate().par_bridge().for_each(|(i, rows)| {
            rows.chunks_mut(num_cols).enumerate().for_each(|(j, row)| {
                let idx = i * chunk_size + j;
                if idx < events.len() {
                    let mut new_byte_lookup_events = Vec::new();
                    let cols: &mut WeierstrassDoubleAssignCols<F, E::BaseField> = row.borrow_mut();
                    match &events[idx].1 {
                        PrecompileEvent::Secp256k1Double(event)
                        | PrecompileEvent::Secp256r1Double(event)
                        | PrecompileEvent::Bn254Double(event)
                        | PrecompileEvent::Bls12381Double(event) => {
                            Self::populate_row(
                                event,
                                cols,
                                &mut new_byte_lookup_events,
                                input.public_values.is_untrusted_programs_enabled,
                            );
                        }
                        _ => unreachable!(),
                    }
                } else {
                    row.copy_from_slice(&dummy_row);
                }
            });
        });

        // Convert the trace to a row major matrix.
        RowMajorMatrix::new(values, num_weierstrass_double_cols::<E::BaseField>())
    }

    fn included(&self, shard: &Self::Record) -> bool {
        if let Some(shape) = shard.shape.as_ref() {
            shape.included::<F, _>(self)
        } else {
            match E::CURVE_TYPE {
                CurveType::Secp256k1 => {
                    !shard.get_precompile_events(SyscallCode::SECP256K1_DOUBLE).is_empty()
                }
                CurveType::Secp256r1 => {
                    !shard.get_precompile_events(SyscallCode::SECP256R1_DOUBLE).is_empty()
                }
                CurveType::Bn254 => {
                    !shard.get_precompile_events(SyscallCode::BN254_DOUBLE).is_empty()
                }
                CurveType::Bls12381 => {
                    !shard.get_precompile_events(SyscallCode::BLS12381_DOUBLE).is_empty()
                }
                _ => panic!("Unsupported curve"),
            }
        }
    }
}

impl<E: EllipticCurve + WeierstrassParameters> WeierstrassDoubleAssignChip<E> {
    pub fn populate_row<F: PrimeField32>(
        event: &EllipticCurveDoubleEvent,
        cols: &mut WeierstrassDoubleAssignCols<F, E::BaseField>,
        new_byte_lookup_events: &mut Vec<ByteLookupEvent>,
        page_prot_enabled: u32,
    ) {
        // Decode affine points.
        let p = &event.p;
        let p = AffinePoint::<E>::from_words_le(p);
        let (p_x, p_y) = (p.x, p.y);

        // Populate basic columns.
        cols.is_real = F::one();
        cols.clk_high = F::from_canonical_u32((event.clk >> 24) as u32);
        cols.clk_low = F::from_canonical_u32((event.clk & 0xFFFFFF) as u32);
        cols.p_ptr.populate(new_byte_lookup_events, event.p_ptr, E::NB_LIMBS as u64 * 2);

        Self::populate_field_ops(new_byte_lookup_events, cols, p_x, p_y);

        // Populate the memory access columns.
        for i in 0..cols.p_access.len() {
            let record = MemoryRecordEnum::Write(event.p_memory_records[i]);
            cols.p_access[i].populate(record, new_byte_lookup_events);
            cols.p_addrs[i].populate(new_byte_lookup_events, event.p_ptr, 8 * i as u64);
        }
        if page_prot_enabled == 1 {
            cols.write_slice_page_prot_access.populate(
                new_byte_lookup_events,
                event.p_ptr,
                event.p_ptr + 8 * (cols.p_addrs.len() - 1) as u64,
                event.clk,
                PROT_READ | PROT_WRITE,
                &event.write_slice_page_prot_access[0],
                &event.write_slice_page_prot_access.get(1).copied(),
                page_prot_enabled,
            );
        }
    }
}

impl<F, E: EllipticCurve + WeierstrassParameters> BaseAir<F> for WeierstrassDoubleAssignChip<E> {
    fn width(&self) -> usize {
        num_weierstrass_double_cols::<E::BaseField>()
    }
}

impl<AB, E: EllipticCurve + WeierstrassParameters> Air<AB> for WeierstrassDoubleAssignChip<E>
where
    AB: SP1CoreAirBuilder,
    Limbs<AB::Var, <E::BaseField as NumLimbs>::Limbs>: Copy,
{
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let local = main.row_slice(0);
        let local: &WeierstrassDoubleAssignCols<AB::Var, E::BaseField> = (*local).borrow();

        let num_words_field_element = E::BaseField::NB_LIMBS / 8;
        let p_x_limbs = builder
            .generate_limbs(&local.p_access[0..num_words_field_element], local.is_real.into());
        let p_y_limbs = builder
            .generate_limbs(&local.p_access[num_words_field_element..], local.is_real.into());
        let p_x: Limbs<AB::Expr, <E::BaseField as NumLimbs>::Limbs> =
            Limbs(p_x_limbs.try_into().expect("failed to convert limbs"));
        let p_y: Limbs<AB::Expr, <E::BaseField as NumLimbs>::Limbs> =
            Limbs(p_y_limbs.try_into().expect("failed to convert limbs"));

        // `a` in the Weierstrass form: y^2 = x^3 + a * x + b.
        let a = E::BaseField::to_limbs_field::<AB::Expr, _>(&E::a_int());

        // slope = slope_numerator / slope_denominator.
        let slope = {
            // slope_numerator = a + (p.x * p.x) * 3.
            {
                local.p_x_squared.eval(builder, &p_x, &p_x, FieldOperation::Mul, local.is_real);

                local.p_x_squared_times_3.eval(
                    builder,
                    &local.p_x_squared.result,
                    &E::BaseField::to_limbs_field::<AB::Expr, _>(&BigUint::from(3u32)),
                    FieldOperation::Mul,
                    local.is_real,
                );

                local.slope_numerator.eval(
                    builder,
                    &a,
                    &local.p_x_squared_times_3.result,
                    FieldOperation::Add,
                    local.is_real,
                );
            };

            // slope_denominator = 2 * y.
            local.slope_denominator.eval(
                builder,
                &E::BaseField::to_limbs_field::<AB::Expr, _>(&BigUint::from(2u32)),
                &p_y,
                FieldOperation::Mul,
                local.is_real,
            );

            local.slope.eval(
                builder,
                &local.slope_numerator.result,
                &local.slope_denominator.result,
                FieldOperation::Div,
                local.is_real,
            );

            &local.slope.result
        };

        // x = slope * slope - (p.x + p.x).
        let x = {
            local.slope_squared.eval(builder, slope, slope, FieldOperation::Mul, local.is_real);
            local.p_x_plus_p_x.eval(builder, &p_x, &p_x, FieldOperation::Add, local.is_real);
            local.x3_ins.eval(
                builder,
                &local.slope_squared.result,
                &local.p_x_plus_p_x.result,
                FieldOperation::Sub,
                local.is_real,
            );
            &local.x3_ins.result
        };

        // y = slope * (p.x - x) - p.y.
        {
            local.p_x_minus_x.eval(builder, &p_x, x, FieldOperation::Sub, local.is_real);
            local.slope_times_p_x_minus_x.eval(
                builder,
                slope,
                &local.p_x_minus_x.result,
                FieldOperation::Mul,
                local.is_real,
            );
            local.y3_ins.eval(
                builder,
                &local.slope_times_p_x_minus_x.result,
                &p_y,
                FieldOperation::Sub,
                local.is_real,
            );
        }

        let modulus = E::BaseField::to_limbs_field::<AB::Expr, AB::F>(&E::BaseField::modulus());
        local.x3_range.eval(builder, &local.x3_ins.result, &modulus, local.is_real);
        local.y3_range.eval(builder, &local.y3_ins.result, &modulus, local.is_real);

        let x3_result_words = limbs_to_words::<AB>(local.x3_ins.result.0.to_vec());
        let y3_result_words = limbs_to_words::<AB>(local.y3_ins.result.0.to_vec());
        let result_words = x3_result_words.into_iter().chain(y3_result_words).collect_vec();

        let p_ptr = SyscallAddrOperation::<AB::F>::eval(
            builder,
            E::NB_LIMBS as u32 * 2,
            local.p_ptr,
            local.is_real.into(),
        );

        // p_addrs[i] = p_ptr + 8 * i
        for i in 0..local.p_addrs.len() {
            AddrAddOperation::<AB::F>::eval(
                builder,
                Word([p_ptr[0].into(), p_ptr[1].into(), p_ptr[2].into(), AB::Expr::zero()]),
                Word::from(8 * i as u64),
                local.p_addrs[i],
                local.is_real.into(),
            );
        }

        builder.eval_memory_access_slice_write(
            local.clk_high,
            local.clk_low.into(),
            &local.p_addrs.iter().map(|addr| addr.value.map(Into::into)).collect_vec(),
            &local.p_access.iter().map(|access| access.memory_access).collect_vec(),
            result_words,
            local.is_real,
        );

        // Fetch the syscall id for the curve type.
        let syscall_id_felt = match E::CURVE_TYPE {
            CurveType::Secp256k1 => {
                AB::F::from_canonical_u32(SyscallCode::SECP256K1_DOUBLE.syscall_id())
            }
            CurveType::Secp256r1 => {
                AB::F::from_canonical_u32(SyscallCode::SECP256R1_DOUBLE.syscall_id())
            }
            CurveType::Bn254 => AB::F::from_canonical_u32(SyscallCode::BN254_DOUBLE.syscall_id()),
            CurveType::Bls12381 => {
                AB::F::from_canonical_u32(SyscallCode::BLS12381_DOUBLE.syscall_id())
            }
            _ => panic!("Unsupported curve"),
        };

        builder.receive_syscall(
            local.clk_high,
            local.clk_low,
            syscall_id_felt,
            p_ptr.map(Into::into),
            [AB::Expr::zero(), AB::Expr::zero(), AB::Expr::zero()].map(Into::into),
            local.is_real,
            InteractionScope::Local,
        );

        AddressSlicePageProtOperation::<AB::F>::eval(
            builder,
            local.clk_high.into(),
            local.clk_low.into(),
            &local.p_ptr.addr.map(Into::into),
            &local.p_addrs[local.p_addrs.len() - 1].value.map(Into::into),
            AB::Expr::from_canonical_u8(PROT_READ | PROT_WRITE),
            &local.write_slice_page_prot_access,
            local.is_real.into(),
        );
    }
}

#[cfg(test)]
pub mod tests {
    use std::sync::Arc;

    use sp1_core_executor::Program;
    use test_artifacts::{
        BLS12381_DOUBLE_ELF, BN254_DOUBLE_ELF, SECP256K1_DOUBLE_ELF, SECP256R1_DOUBLE_ELF,
    };

    use crate::{
        io::SP1Stdin,
        utils::{run_test, setup_logger},
    };

    #[tokio::test]
    async fn test_secp256k1_double_simple() {
        setup_logger();
        let program = Arc::new(Program::from(&SECP256K1_DOUBLE_ELF).unwrap());
        let stdin = SP1Stdin::new();
        run_test(program, stdin).await.unwrap();
    }

    #[tokio::test]
    async fn test_secp256r1_double_simple() {
        setup_logger();
        let program = Arc::new(Program::from(&SECP256R1_DOUBLE_ELF).unwrap());
        let stdin = SP1Stdin::new();
        run_test(program, stdin).await.unwrap();
    }

    #[tokio::test]
    async fn test_bn254_double_simple() {
        setup_logger();
        let program = Arc::new(Program::from(&BN254_DOUBLE_ELF).unwrap());
        let stdin = SP1Stdin::new();
        run_test(program, stdin).await.unwrap();
    }

    #[tokio::test]
    async fn test_bls12381_double_simple() {
        setup_logger();
        let program = Arc::new(Program::from(&BLS12381_DOUBLE_ELF).unwrap());
        let stdin = SP1Stdin::new();
        run_test(program, stdin).await.unwrap();
    }
}
