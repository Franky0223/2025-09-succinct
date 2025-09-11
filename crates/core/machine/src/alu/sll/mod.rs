use core::{
    borrow::{Borrow, BorrowMut},
    mem::size_of,
};

use hashbrown::HashMap;
use itertools::Itertools;
use slop_air::{Air, AirBuilder, BaseAir};
use slop_algebra::{AbstractField, Field, PrimeField, PrimeField32};
use slop_matrix::{dense::RowMajorMatrix, Matrix};
use slop_maybe_rayon::prelude::{ParallelIterator, ParallelSlice};
use sp1_core_executor::{
    events::{AluEvent, ByteLookupEvent, ByteRecord},
    ALUTypeRecord, ByteOpcode, ExecutionRecord, Opcode, Program, CLK_INC, PC_INC,
};
use sp1_derive::AlignedBorrow;
use sp1_hypercube::{air::MachineAir, Word};
use sp1_primitives::consts::{u32_to_u16_limbs, u64_to_u16_limbs, WORD_SIZE};

use crate::{
    adapter::{
        register::alu_type::{ALUTypeReader, ALUTypeReaderInput},
        state::{CPUState, CPUStateInput},
    },
    air::{SP1CoreAirBuilder, SP1Operation},
    operations::{U16MSBOperation, U16MSBOperationInput},
    utils::{next_multiple_of_32, pad_rows_fixed},
};

/// The number of main trace columns for `ShiftLeft`.
pub const NUM_SHIFT_LEFT_COLS: usize = size_of::<ShiftLeftCols<u8>>();

/// The number of bits in a byte.
pub const BYTE_SIZE: usize = 8;

/// A chip that implements bitwise operations for the opcodes SLL and SLLI.
#[derive(Default)]
pub struct ShiftLeft;

/// The column layout for the chip.
#[derive(AlignedBorrow, Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct ShiftLeftCols<T> {
    /// The current shard, timestamp, program counter of the CPU.
    pub state: CPUState<T>,

    /// The adapter to read program and register information.
    pub adapter: ALUTypeReader<T>,

    /// The output operand.
    pub a: Word<T>,

    /// The lowerst byte of `c`.
    pub c_bits: [T; 6],

    /// v01 = (c0 + 1) * (3c1 + 1)
    pub v_01: T,

    /// v012 = (c0 + 1) * (3c1 + 1) * (15c2 + 1)
    pub v_012: T,

    /// v012 * c3
    pub v_0123: T,

    /// Flags representing c4 + 2c5.
    pub shift_u16: [T; 4],

    /// The lower bits of each limb.
    pub lower_limb: Word<T>,

    /// The higher bits of each limb.
    pub higher_limb: Word<T>,

    /// The limb results.
    pub limb_result: Word<T>,

    /// The most significant byte of the result of SLLW.
    pub sllw_msb: U16MSBOperation<T>,

    /// If the opcode is SLL.
    pub is_sll: T,

    /// If the opcode is SLLW.
    pub is_sllw: T,

    /// If the opcode is SLLW and immediate.
    pub is_sllw_imm: T,
}

impl<F: PrimeField32> MachineAir<F> for ShiftLeft {
    type Record = ExecutionRecord;

    type Program = Program;

    fn name(&self) -> String {
        "ShiftLeft".to_string()
    }

    fn num_rows(&self, input: &Self::Record) -> Option<usize> {
        let nb_rows =
            next_multiple_of_32(input.shift_left_events.len(), input.fixed_log2_rows::<F, _>(self));
        Some(nb_rows)
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        _: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        // Generate the trace rows for each event.
        let mut rows: Vec<[F; NUM_SHIFT_LEFT_COLS]> = vec![];
        let shift_left_events = input.shift_left_events.clone();
        for event in shift_left_events.iter() {
            let mut row = [F::zero(); NUM_SHIFT_LEFT_COLS];
            let cols: &mut ShiftLeftCols<F> = row.as_mut_slice().borrow_mut();
            let mut blu = Vec::new();
            cols.adapter.populate(&mut blu, event.1);
            self.event_to_row(&event.0, &event.1, cols, &mut blu);
            cols.state.populate(&mut blu, event.0.clk, event.0.pc);
            rows.push(row);
        }

        // Pad the trace to a power of two depending on the proof shape in `input`.
        pad_rows_fixed(
            &mut rows,
            || [F::zero(); NUM_SHIFT_LEFT_COLS],
            input.fixed_log2_rows::<F, _>(self),
        );

        assert_eq!(rows.len(), <ShiftLeft as MachineAir<F>>::num_rows(self, input).unwrap());

        // Convert the trace to a row major matrix.
        let mut trace = RowMajorMatrix::new(
            rows.into_iter().flatten().collect::<Vec<_>>(),
            NUM_SHIFT_LEFT_COLS,
        );

        // Create the template for the padded rows. These are fake rows that don't fail on some
        // sanity checks.
        let padded_row_template = {
            let mut row = [F::zero(); NUM_SHIFT_LEFT_COLS];
            let cols: &mut ShiftLeftCols<F> = row.as_mut_slice().borrow_mut();
            cols.v_01 = F::one();
            cols.v_012 = F::one();
            cols.v_0123 = F::one();
            row
        };
        debug_assert!(padded_row_template.len() == NUM_SHIFT_LEFT_COLS);
        for i in input.shift_left_events.len() * NUM_SHIFT_LEFT_COLS..trace.values.len() {
            trace.values[i] = padded_row_template[i % NUM_SHIFT_LEFT_COLS];
        }

        trace
    }

    fn generate_dependencies(&self, input: &Self::Record, output: &mut Self::Record) {
        let chunk_size = std::cmp::max(input.shift_left_events.len() / num_cpus::get(), 1);

        let blu_batches = input
            .shift_left_events
            .par_chunks(chunk_size)
            .map(|events| {
                let mut blu: HashMap<ByteLookupEvent, usize> = HashMap::new();
                events.iter().for_each(|event| {
                    let mut row = [F::zero(); NUM_SHIFT_LEFT_COLS];
                    let cols: &mut ShiftLeftCols<F> = row.as_mut_slice().borrow_mut();
                    cols.adapter.populate(&mut blu, event.1);
                    self.event_to_row(&event.0, &event.1, cols, &mut blu);
                    cols.state.populate(&mut blu, event.0.clk, event.0.pc);
                });
                blu
            })
            .collect::<Vec<_>>();

        output.add_byte_lookup_events_from_maps(blu_batches.iter().collect_vec());
    }

    fn included(&self, shard: &Self::Record) -> bool {
        if let Some(shape) = shard.shape.as_ref() {
            shape.included::<F, _>(self)
        } else {
            !shard.shift_left_events.is_empty()
        }
    }
}

impl ShiftLeft {
    /// Create a row from an event.
    fn event_to_row<F: PrimeField>(
        &self,
        event: &AluEvent,
        record: &ALUTypeRecord,
        cols: &mut ShiftLeftCols<F>,
        blu: &mut impl ByteRecord,
    ) {
        let c = u64_to_u16_limbs(event.c)[0];

        if event.opcode == Opcode::SLLW {
            let sllw_val = ((event.b as i64) << (c & 0x1f)) as u32;
            let sllw_limbs = u32_to_u16_limbs(sllw_val);
            cols.sllw_msb.populate_msb(blu, sllw_limbs[1]);
        }

        cols.a = Word::from(event.a);
        let is_sll = event.opcode == Opcode::SLL;
        cols.is_sll = F::from_bool(is_sll);

        cols.is_sllw = F::from_bool(event.opcode == Opcode::SLLW);
        cols.is_sllw_imm = F::from_bool(event.opcode == Opcode::SLLW && record.is_imm);

        for i in 0..6 {
            cols.c_bits[i] = F::from_canonical_u16((c >> i) & 1);
        }
        blu.add_bit_range_check(c >> 6, 10);

        cols.v_01 = F::from_canonical_u16(1 << (c & 3));
        cols.v_012 = F::from_canonical_u16(1 << (c & 7));
        cols.v_0123 = F::from_canonical_u16(1 << (c & 15));

        let shift_amount = ((c >> 4) & 1) + 2 * ((c >> 5) & 1) * (is_sll as u16);

        let mut shift = [0u16; 4];
        for i in 0..4 {
            if i == shift_amount as usize {
                shift[i] = 1;
            }
        }

        let b = u64_to_u16_limbs(event.b);
        let bit_shift = (c & 0xF) as u8;
        for i in 0..WORD_SIZE {
            let limb = b[i] as u32;
            let lower_limb = (limb & ((1 << (16 - bit_shift)) - 1)) as u16;
            let higher_limb = (limb >> (16 - bit_shift)) as u16;
            cols.lower_limb[i] = F::from_canonical_u16(lower_limb);
            cols.higher_limb[i] = F::from_canonical_u16(higher_limb);
            blu.add_bit_range_check(lower_limb, 16 - bit_shift);
            blu.add_bit_range_check(higher_limb, bit_shift);
        }

        for i in 0..WORD_SIZE {
            cols.limb_result[i] = cols.lower_limb[i] * F::from_canonical_u32(1u32 << bit_shift);
            if i != 0 {
                cols.limb_result[i] += cols.higher_limb[i - 1];
            }
        }

        cols.shift_u16 = shift.map(|x| F::from_canonical_u16(x));
    }
}

impl<F> BaseAir<F> for ShiftLeft {
    fn width(&self) -> usize {
        NUM_SHIFT_LEFT_COLS
    }
}

impl<AB> Air<AB> for ShiftLeft
where
    AB: SP1CoreAirBuilder,
{
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let local = main.row_slice(0);
        let local: &ShiftLeftCols<AB::Var> = (*local).borrow();

        // SAFETY: All selectors `is_sll`, `is_sllw` are checked to be boolean.
        // Each "real" row has exactly one selector turned on, as `is_real = is_sll + is_sllw` is
        // boolean. All interactions are done with multiplicity `is_real`.
        // Therefore, the `opcode` matches the corresponding opcode.
        let is_real = local.is_sll + local.is_sllw;
        builder.assert_bool(is_real.clone());
        builder.assert_bool(local.is_sll);
        builder.assert_bool(local.is_sllw);

        // Check that `local.c_bits` are the 6 lowest bits of `c`.
        for i in 0..6 {
            builder.assert_bool(local.c_bits[i]);
        }
        let mut c_lower_bits = AB::Expr::zero();
        let mut bit_shift = AB::Expr::zero();
        for i in 0..6 {
            c_lower_bits = c_lower_bits + local.c_bits[i] * AB::F::from_canonical_u32(1 << i);
            if i == 3 {
                bit_shift = c_lower_bits.clone();
            }
        }
        let inverse_64 = AB::F::from_canonical_u32(64).inverse();
        builder.send_byte(
            AB::F::from_canonical_u32(ByteOpcode::Range as u32),
            (local.adapter.c()[0] - c_lower_bits) * inverse_64,
            AB::Expr::from_canonical_u32(10),
            AB::Expr::zero(),
            is_real.clone(),
        );

        // Check that `shift_u16` represents the boolean flag for the u16 limb shifts.
        for i in 0..WORD_SIZE {
            builder.when(local.shift_u16[i]).assert_eq(
                local.c_bits[4] + local.c_bits[5] * AB::F::from_canonical_u32(2) * local.is_sll,
                AB::Expr::from_canonical_u32(i as u32),
            );
            builder.assert_bool(local.shift_u16[i]);
        }

        builder.when(is_real.clone()).assert_eq(
            local.shift_u16[0] + local.shift_u16[1] + local.shift_u16[2] + local.shift_u16[3],
            AB::Expr::from_canonical_u32(1),
        );

        let one = AB::F::from_canonical_u32(1);
        let three = AB::F::from_canonical_u32(3);
        let fifteen = AB::F::from_canonical_u32(15);
        let two_fifty_five = AB::F::from_canonical_u32(255);
        builder.assert_eq(local.v_01, (local.c_bits[0] + one) * (local.c_bits[1] * three + one));
        builder.assert_eq(local.v_012, local.v_01 * (local.c_bits[2] * fifteen + one));
        builder.assert_eq(local.v_0123, local.v_012 * (local.c_bits[3] * two_fifty_five + one));

        for i in 0..WORD_SIZE {
            let limb = local.adapter.b()[i];
            // Check that `lower_limb < 2^(16 - bit_shift)`
            builder.send_byte(
                AB::F::from_canonical_u32(ByteOpcode::Range as u32),
                local.lower_limb[i],
                AB::Expr::from_canonical_u32(16) - bit_shift.clone(),
                AB::Expr::zero(),
                is_real.clone(),
            );
            // Check that `higher_limb < 2^(bit_shift)`
            builder.send_byte(
                AB::F::from_canonical_u32(ByteOpcode::Range as u32),
                local.higher_limb[i],
                bit_shift.clone(),
                AB::Expr::zero(),
                is_real.clone(),
            );
            // Check that `limb == higher_limb * 2^(16 - bit_shift) + lower_limb`
            // Multiply `2^(bit_shift)` to the equation to avoid populating `2^(16 - bit_shift)`.
            // This is possible, since `2^(bit_shift)` is not zero.
            builder.assert_eq(
                limb * local.v_0123,
                local.higher_limb[i] * AB::Expr::from_canonical_u32(1 << 16)
                    + local.lower_limb[i] * local.v_0123,
            );
        }

        // Compute the limb result based on the lower limbs and higher limbs.
        for i in 0..WORD_SIZE {
            let mut limb_result = local.lower_limb[i] * local.v_0123;
            if i != 0 {
                limb_result = limb_result.clone() + local.higher_limb[i - 1];
            }
            builder.assert_eq(local.limb_result[i], limb_result);
        }

        // Perform the limb shifts based on `shift_u16` boolean flags.
        for i in 0..WORD_SIZE {
            for j in 0..WORD_SIZE {
                if j < i {
                    builder.when(local.is_sll).when(local.shift_u16[i]).assert_zero(local.a[j]);
                } else {
                    builder
                        .when(local.is_sll)
                        .when(local.shift_u16[i])
                        .assert_eq(local.a[j], local.limb_result[j - i]);
                }
            }
        }

        for i in 0..WORD_SIZE / 2 {
            for j in 0..WORD_SIZE / 2 {
                if j < i {
                    builder.when(local.is_sllw).when(local.shift_u16[i]).assert_zero(local.a[j]);
                } else {
                    builder
                        .when(local.is_sllw)
                        .when(local.shift_u16[i])
                        .assert_eq(local.a[j], local.limb_result[j - i]);
                }
            }
        }
        let u16_max = AB::F::from_canonical_u16(u16::MAX);
        for i in WORD_SIZE / 2..WORD_SIZE {
            builder.when(local.is_sllw).assert_eq(local.sllw_msb.msb * u16_max, local.a[i]);
        }

        U16MSBOperation::<AB::F>::eval(
            builder,
            U16MSBOperationInput::new(local.a[1].into(), local.sllw_msb, local.is_sllw.into()),
        );

        let opcode = local.is_sll * AB::F::from_canonical_u32(Opcode::SLL as u32)
            + local.is_sllw * AB::F::from_canonical_u32(Opcode::SLLW as u32);

        // Compute instruction field constants for each opcode
        let funct3 = local.is_sll * AB::Expr::from_canonical_u8(Opcode::SLL.funct3().unwrap())
            + local.is_sllw * AB::Expr::from_canonical_u8(Opcode::SLLW.funct3().unwrap());
        let funct7 = local.is_sll * AB::Expr::from_canonical_u8(Opcode::SLL.funct7().unwrap_or(0))
            + local.is_sllw * AB::Expr::from_canonical_u8(Opcode::SLLW.funct7().unwrap_or(0));

        let (sll_base, sll_imm) = Opcode::SLL.base_opcode();
        let sll_imm = sll_imm.expect("SLL immediate opcode not found");
        let (sllw_base, sllw_imm) = Opcode::SLLW.base_opcode();
        let sllw_imm = sllw_imm.expect("SLLW immediate opcode not found");

        let imm_base_difference = sll_base.checked_sub(sll_imm).unwrap();
        assert!(imm_base_difference == sllw_base.checked_sub(sllw_imm).unwrap());

        let sll_base_expr = AB::Expr::from_canonical_u32(sll_base);
        let sllw_base_expr = AB::Expr::from_canonical_u32(sllw_base);

        // Start with register opcode, if it's immediate, subtract the difference
        let calculated_base_opcode = local.is_sll * sll_base_expr + local.is_sllw * sllw_base_expr
            - AB::Expr::from_canonical_u32(imm_base_difference) * local.adapter.imm_c;

        let sll_instr_type = Opcode::SLL.instruction_type().0 as u32;
        let sll_instr_type_imm =
            Opcode::SLL.instruction_type().1.expect("SLL immediate instruction type not found")
                as u32;
        let sllw_instr_type = Opcode::SLLW.instruction_type().0 as u32;
        let sllw_instr_type_imm =
            Opcode::SLLW.instruction_type().1.expect("SLLW immediate instruction type not found")
                as u32;

        let instr_type_difference = sll_instr_type.checked_sub(sll_instr_type_imm).unwrap();
        let w_instr_imm_adjustment = sll_instr_type_imm.checked_sub(sllw_instr_type_imm).unwrap();
        assert_eq!(
            sllw_instr_type.checked_sub(sllw_instr_type_imm).unwrap(),
            instr_type_difference + w_instr_imm_adjustment,
        );

        builder.assert_eq(local.is_sllw_imm, local.is_sllw * local.adapter.imm_c);

        let calculated_instr_type = local.is_sll * AB::Expr::from_canonical_u32(sll_instr_type)
            + local.is_sllw * AB::Expr::from_canonical_u32(sllw_instr_type)
            - (AB::Expr::from_canonical_u32(instr_type_difference) * local.adapter.imm_c
                + AB::Expr::from_canonical_u32(w_instr_imm_adjustment) * local.is_sllw_imm);

        // Constrain the CPU state.
        // The program counter and timestamp increment by `4` and `8`.
        <CPUState<AB::F> as SP1Operation<AB>>::eval(
            builder,
            CPUStateInput::new(
                local.state,
                [
                    local.state.pc[0] + AB::F::from_canonical_u32(PC_INC),
                    local.state.pc[1].into(),
                    local.state.pc[2].into(),
                ],
                AB::Expr::from_canonical_u32(CLK_INC),
                is_real.clone(),
            ),
        );

        // Constrain the program and register reads.
        let alu_reader_input = ALUTypeReaderInput::<AB, AB::Expr>::new(
            local.state.clk_high::<AB>(),
            local.state.clk_low::<AB>(),
            local.state.pc,
            opcode,
            [calculated_instr_type, calculated_base_opcode, funct3, funct7],
            local.a.map(|x| x.into()),
            local.adapter,
            is_real.clone(),
        );
        ALUTypeReader::<AB::F>::eval(builder, alu_reader_input);
    }
}

//     use std::borrow::BorrowMut;

//     use crate::{
//         alu::ShiftLeftCols,
//         io::SP1Stdin,
//         riscv::RiscvAir,
//         utils::{run_malicious_test, run_test_machine, setup_test_machine},
//     };
//     use sp1_primitives::SP1Field;
//     use slop_matrix::dense::RowMajorMatrix;
//     use rand::{thread_rng, Rng};
//     use sp1_core_executor::{
//         events::{AluEvent, MemoryRecordEnum},
//         ExecutionRecord, Instruction, Opcode, Program,
//     };
//     use sp1_hypercube::{
//         air::{MachineAir, SP1_PROOF_NUM_PV_ELTS},
//         koala_bear_poseidon2::SP1CoreJaggedConfig,
//         chip_name, Chip, CpuProver, MachineProver, StarkMachine, Val,
//     };

//     use super::ShiftLeft;

//     #[test]
//     fn generate_trace() {
//         let mut shard = ExecutionRecord::default();
//         shard.shift_left_events = vec![AluEvent::new(0, Opcode::SLL, 16, 8, 1, false)];
//         let chip = ShiftLeft::default();
//         let trace: RowMajorMatrix<SP1Field> =
//             chip.generate_trace(&shard, &mut ExecutionRecord::default());
//         println!("{:?}", trace.values)
//     }

//     #[test]
//     fn prove_koalabear() {
//         let mut shift_events: Vec<AluEvent> = Vec::new();
//         let shift_instructions: Vec<(Opcode, u32, u32, u32)> = vec![
//             (Opcode::SLL, 0x00000002, 0x00000001, 1),
//             (Opcode::SLL, 0x00000080, 0x00000001, 7),
//             (Opcode::SLL, 0x00004000, 0x00000001, 14),
//             (Opcode::SLL, 0x80000000, 0x00000001, 31),
//             (Opcode::SLL, 0xffffffff, 0xffffffff, 0),
//             (Opcode::SLL, 0xfffffffe, 0xffffffff, 1),
//             (Opcode::SLL, 0xffffff80, 0xffffffff, 7),
//             (Opcode::SLL, 0xffffc000, 0xffffffff, 14),
//             (Opcode::SLL, 0x80000000, 0xffffffff, 31),
//             (Opcode::SLL, 0x21212121, 0x21212121, 0),
//             (Opcode::SLL, 0x42424242, 0x21212121, 1),
//             (Opcode::SLL, 0x90909080, 0x21212121, 7),
//             (Opcode::SLL, 0x48484000, 0x21212121, 14),
//             (Opcode::SLL, 0x80000000, 0x21212121, 31),
//             (Opcode::SLL, 0x21212121, 0x21212121, 0xffffffe0),
//             (Opcode::SLL, 0x42424242, 0x21212121, 0xffffffe1),
//             (Opcode::SLL, 0x90909080, 0x21212121, 0xffffffe7),
//             (Opcode::SLL, 0x48484000, 0x21212121, 0xffffffee),
//             (Opcode::SLL, 0x00000000, 0x21212120, 0xffffffff),
//         ];
//         for t in shift_instructions.iter() {
//             shift_events.push(AluEvent::new(0, t.0, t.1, t.2, t.3, false));
//         }

//         // Append more events until we have 1000 tests.
//         for _ in 0..(1000 - shift_instructions.len()) {
//             //shift_events.push(AluEvent::new(0, 0, Opcode::SLL, 14, 8, 6));
//         }

//         let mut shard = ExecutionRecord::default();
//         shard.shift_left_events = shift_events;

//         // Run setup.
//         let air = ShiftLeft::default();
//         let config = SP1CoreJaggedConfig::new();
//         let chip = Chip::new(air);
//         let (pk, vk) = setup_test_machine(StarkMachine::new(
//             config.clone(),
//             vec![chip],
//             SP1_PROOF_NUM_PV_ELTS,
//             true,
//         ));

//         // Run the test.
//         let air = ShiftLeft::default();
//         let chip: Chip<SP1Field, ShiftLeft> = Chip::new(air);
//         let machine = StarkMachine::new(config.clone(), vec![chip], SP1_PROOF_NUM_PV_ELTS, true);
//         run_test_machine::<SP1CoreJaggedConfig, ShiftLeft>(vec![shard], machine, pk,
// vk).unwrap();     }

//     #[test]
//     fn test_malicious_sll() {
//         const NUM_TESTS: usize = 5;

//         for _ in 0..NUM_TESTS {
//             let op_a = thread_rng().gen_range(0..u32::MAX);
//             let op_b = thread_rng().gen_range(0..u32::MAX);
//             let op_c = thread_rng().gen_range(0..u32::MAX);

//             let correct_op_a = op_b << (op_c & 0x1F);

//             assert!(op_a != correct_op_a);

//             let instructions = vec![
//                 Instruction::new(Opcode::SLL, 5, op_b, op_c, true, true),
//                 Instruction::new(Opcode::ADD, 10, 0, 0, false, false),
//             ];

//             let program = Program::new(instructions, 0, 0);
//             let stdin = SP1Stdin::new();

//             type P = CpuProver<SP1CoreJaggedConfig, RiscvAir<SP1Field>>;

//             let malicious_trace_pv_generator =
//                 move |prover: &P,
//                       record: &mut ExecutionRecord|
//                       -> Vec<(String, RowMajorMatrix<Val<SP1CoreJaggedConfig>>)> {
//                     let mut malicious_record = record.clone();
//                     malicious_record.cpu_events[0].a = op_a as u32;
//                     if let Some(MemoryRecordEnum::Write(mut write_record)) =
//                         malicious_record.cpu_events[0].a_record
//                     {
//                         write_record.value = op_a as u32;
//                     }
//                     let mut traces = prover.generate_traces(&malicious_record);
//                     let shift_left_chip_name = chip_name!(ShiftLeft, SP1Field);
//                     for (name, trace) in traces.iter_mut() {
//                         if *name == shift_left_chip_name {
//                             let first_row = trace.row_mut(0);
//                             let first_row: &mut ShiftLeftCols<SP1Field> = first_row.borrow_mut();
//                             first_row.a = op_a.into();
//                         }
//                     }

//                     traces
//                 };

//             let result =
//                 run_malicious_test::<P>(program, stdin, Box::new(malicious_trace_pv_generator));
//             assert!(result.is_err() && result.unwrap_err().is_constraints_failing());
//         }
//     }
// }
