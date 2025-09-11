use core::{
    borrow::{Borrow, BorrowMut},
    mem::size_of,
};

use hashbrown::HashMap;
use itertools::Itertools;
use slop_air::{Air, BaseAir};
use slop_algebra::{AbstractField, PrimeField32};
use slop_matrix::{dense::RowMajorMatrix, Matrix};
use slop_maybe_rayon::prelude::*;
use sp1_core_executor::{
    events::{AluEvent, ByteLookupEvent, ByteRecord},
    ExecutionRecord, Opcode, Program, CLK_INC, PC_INC,
};
use sp1_derive::AlignedBorrow;
use sp1_hypercube::{air::MachineAir, Word};

use crate::{
    adapter::{
        register::alu_type::{ALUTypeReader, ALUTypeReaderInput},
        state::{CPUState, CPUStateInput},
    },
    air::{SP1CoreAirBuilder, SP1Operation},
    operations::{LtOperationSigned, LtOperationSignedInput},
    utils::{next_multiple_of_32, zeroed_f_vec},
};

/// The number of main trace columns for `LtChip`.
pub const NUM_LT_COLS: usize = size_of::<LtCols<u8>>();

/// A chip that implements comparison operations for the opcodes SLT and SLTU.
#[derive(Default)]
pub struct LtChip;

/// The column layout for the chip.
#[derive(AlignedBorrow, Default, Clone, Copy)]
#[repr(C)]
pub struct LtCols<T> {
    /// The current shard, timestamp, program counter of the CPU.
    pub state: CPUState<T>,

    /// The adapter to read program and register information.
    pub adapter: ALUTypeReader<T>,

    /// If the opcode is SLT.
    pub is_slt: T,

    /// If the opcode is SLTU.
    pub is_sltu: T,

    /// Instance of `LtOperationSigned` to handle comparison logic in `LtChip`'s ALU operations.
    pub lt_operation: LtOperationSigned<T>,
}

impl LtCols<u32> {
    pub fn from_trace_row<F: PrimeField32>(row: &[F]) -> Self {
        let sized: [u32; NUM_LT_COLS] =
            row.iter().map(|x| x.as_canonical_u32()).collect::<Vec<u32>>().try_into().unwrap();
        *sized.as_slice().borrow()
    }
}

impl<F: PrimeField32> MachineAir<F> for LtChip {
    type Record = ExecutionRecord;

    type Program = Program;

    fn name(&self) -> String {
        "Lt".to_string()
    }

    fn num_rows(&self, input: &Self::Record) -> Option<usize> {
        let nb_rows =
            next_multiple_of_32(input.lt_events.len(), input.fixed_log2_rows::<F, _>(self));
        Some(nb_rows)
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        _: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        // Generate the trace rows for each event.
        let nb_rows = input.lt_events.len();
        let padded_nb_rows = <LtChip as MachineAir<F>>::num_rows(self, input).unwrap();
        let mut values = zeroed_f_vec(padded_nb_rows * NUM_LT_COLS);
        let chunk_size = std::cmp::max((nb_rows + 1) / num_cpus::get(), 1);

        values.chunks_mut(chunk_size * NUM_LT_COLS).enumerate().par_bridge().for_each(
            |(i, rows)| {
                rows.chunks_mut(NUM_LT_COLS).enumerate().for_each(|(j, row)| {
                    let idx = i * chunk_size + j;
                    let cols: &mut LtCols<F> = row.borrow_mut();

                    if idx < nb_rows {
                        let mut byte_lookup_events = Vec::new();
                        let event = &input.lt_events[idx];
                        cols.adapter.populate(&mut byte_lookup_events, event.1);
                        self.event_to_row(&event.0, cols, &mut byte_lookup_events);
                        cols.state.populate(&mut byte_lookup_events, event.0.clk, event.0.pc);
                    }
                });
            },
        );

        // Convert the trace to a row major matrix.

        RowMajorMatrix::new(values, NUM_LT_COLS)
    }

    fn generate_dependencies(&self, input: &Self::Record, output: &mut Self::Record) {
        let chunk_size = std::cmp::max(input.lt_events.len() / num_cpus::get(), 1);

        let blu_batches = input
            .lt_events
            .par_chunks(chunk_size)
            .map(|events| {
                let mut blu: HashMap<ByteLookupEvent, usize> = HashMap::new();
                events.iter().for_each(|event| {
                    let mut row = [F::zero(); NUM_LT_COLS];
                    let cols: &mut LtCols<F> = row.as_mut_slice().borrow_mut();
                    cols.adapter.populate(&mut blu, event.1);
                    self.event_to_row(&event.0, cols, &mut blu);
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
            !shard.lt_events.is_empty()
        }
    }
}

impl LtChip {
    /// Create a row from an event.
    fn event_to_row<F: PrimeField32>(
        &self,
        event: &AluEvent,
        cols: &mut LtCols<F>,
        blu: &mut impl ByteRecord,
    ) {
        cols.lt_operation.populate_signed(
            blu,
            event.a,
            event.b,
            event.c,
            event.opcode == Opcode::SLT,
        );

        cols.is_slt = F::from_bool(event.opcode == Opcode::SLT);
        cols.is_sltu = F::from_bool(event.opcode == Opcode::SLTU);
    }
}

impl<F> BaseAir<F> for LtChip {
    fn width(&self) -> usize {
        NUM_LT_COLS
    }
}

impl<AB> Air<AB> for LtChip
where
    AB: SP1CoreAirBuilder,
{
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let local = main.row_slice(0);
        let local: &LtCols<AB::Var> = (*local).borrow();

        // SAFETY: All selectors `is_slt`, `is_sltu` are checked to be boolean.
        // Each "real" row has exactly one selector turned on, as `is_real = is_slt + is_sltu` is
        // boolean. Therefore, the `opcode` matches the corresponding opcode.
        let is_real = local.is_slt + local.is_sltu;
        builder.assert_bool(local.is_slt);
        builder.assert_bool(local.is_sltu);
        builder.assert_bool(is_real.clone());

        // Evaluate the LT operation.
        <LtOperationSigned<AB::F> as SP1Operation<AB>>::eval(
            builder,
            LtOperationSignedInput::<AB>::new(
                local.adapter.b().map(|x| x.into()),
                local.adapter.c().map(|x| x.into()),
                local.lt_operation,
                local.is_slt.into(),
                is_real.clone(),
            ),
        );

        // Constrain the state of the CPU.
        // The program counter and timestamp increment by `4` and `8`.
        <CPUState<AB::F> as SP1Operation<AB>>::eval(
            builder,
            CPUStateInput {
                cols: local.state,
                next_pc: [
                    local.state.pc[0] + AB::F::from_canonical_u32(PC_INC),
                    local.state.pc[1].into(),
                    local.state.pc[2].into(),
                ],
                clk_increment: AB::Expr::from_canonical_u32(CLK_INC),
                is_real: is_real.clone(),
            },
        );

        // Get the opcode for the operation.
        let opcode = local.is_slt * AB::F::from_canonical_u32(Opcode::SLT as u32)
            + local.is_sltu * AB::F::from_canonical_u32(Opcode::SLTU as u32);

        // Compute instruction field constants for each opcode
        let funct3 = local.is_slt * AB::Expr::from_canonical_u8(Opcode::SLT.funct3().unwrap())
            + local.is_sltu * AB::Expr::from_canonical_u8(Opcode::SLTU.funct3().unwrap());
        let funct7 = local.is_slt * AB::Expr::from_canonical_u8(Opcode::SLT.funct7().unwrap_or(0))
            + local.is_sltu * AB::Expr::from_canonical_u8(Opcode::SLTU.funct7().unwrap_or(0));

        let (slt_base, slt_imm) = Opcode::SLT.base_opcode();
        let slt_imm = slt_imm.expect("SLT immediate opcode not found");
        let (sltu_base, sltu_imm) = Opcode::SLTU.base_opcode();
        let sltu_imm = sltu_imm.expect("SLTU immediate opcode not found");

        let imm_base_difference = slt_base.checked_sub(slt_imm).unwrap();
        assert_eq!(imm_base_difference, sltu_base.checked_sub(sltu_imm).unwrap());

        let slt_base_expr = AB::Expr::from_canonical_u32(slt_base);
        let sltu_base_expr = AB::Expr::from_canonical_u32(sltu_base);

        let calculated_base_opcode = local.is_slt * slt_base_expr + local.is_sltu * sltu_base_expr
            - AB::Expr::from_canonical_u32(imm_base_difference) * local.adapter.imm_c;

        let slt_instr_type = Opcode::SLT.instruction_type().0 as u32;
        let slt_instr_type_imm =
            Opcode::SLT.instruction_type().1.expect("SLT immediate instruction type not found")
                as u32;
        let sltu_instr_type = Opcode::SLTU.instruction_type().0 as u32;
        let sltu_instr_type_imm =
            Opcode::SLTU.instruction_type().1.expect("SLTU immediate instruction type not found")
                as u32;

        let instr_type_difference = slt_instr_type.checked_sub(slt_instr_type_imm).unwrap();
        assert_eq!(
            instr_type_difference,
            sltu_instr_type.checked_sub(sltu_instr_type_imm).unwrap()
        );

        let calculated_instr_type = local.is_slt * AB::Expr::from_canonical_u32(slt_instr_type)
            + local.is_sltu * AB::Expr::from_canonical_u32(sltu_instr_type)
            - AB::Expr::from_canonical_u32(instr_type_difference) * local.adapter.imm_c;

        // Constrain the program and register reads.
        let alu_reader_input = ALUTypeReaderInput::<AB, AB::Expr>::new(
            local.state.clk_high::<AB>(),
            local.state.clk_low::<AB>(),
            local.state.pc,
            opcode,
            [calculated_instr_type, calculated_base_opcode, funct3, funct7],
            Word::extend_var::<AB>(local.lt_operation.result.u16_compare_operation.bit),
            local.adapter,
            is_real,
        );
        ALUTypeReader::<AB::F>::eval(builder, alu_reader_input);
    }
}

// #[cfg(test)]
// mod tests {
//     #![allow(clippy::print_stdout)]

//     use std::borrow::BorrowMut;

//     use crate::{
//         alu::LtCols,
//         io::SP1Stdin,
//         riscv::RiscvAir,
//         utils::{run_malicious_test, run_test_machine, setup_test_machine},
//     };
//     use sp1_primitives::SP1Field;
//     use slop_algebra::AbstractField;
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

//     use super::LtChip;

//     #[test]
//     fn generate_trace() {
//         let mut shard = ExecutionRecord::default();
//         shard.lt_events = vec![AluEvent::new(0, Opcode::SLT, 0, 3, 2, false)];
//         let chip = LtChip::default();
//         let generate_trace = chip.generate_trace(&shard, &mut ExecutionRecord::default());
//         let trace: RowMajorMatrix<SP1Field> = generate_trace;
//         println!("{:?}", trace.values)
//     }

//     fn prove_koalabear_template(shard: ExecutionRecord) {
//         // Run setup.
//         let air = LtChip::default();
//         let config = SP1CoreJaggedConfig::new();
//         let chip = Chip::new(air);
//         let (pk, vk) = setup_test_machine(StarkMachine::new(
//             config.clone(),
//             vec![chip],
//             SP1_PROOF_NUM_PV_ELTS,
//             true,
//         ));

//         // Run the test.
//         let air = LtChip::default();
//         let chip: Chip<SP1Field, LtChip> = Chip::new(air);
//         let machine = StarkMachine::new(config.clone(), vec![chip], SP1_PROOF_NUM_PV_ELTS, true);
//         run_test_machine::<SP1CoreJaggedConfig, LtChip>(vec![shard], machine, pk, vk).unwrap();
//     }

//     #[test]
//     fn prove_koalabear_slt() {
//         let mut shard = ExecutionRecord::default();

//         const NEG_3: u32 = 0b11111111111111111111111111111101;
//         const NEG_4: u32 = 0b11111111111111111111111111111100;
//         shard.lt_events = vec![
//             // 0 == 3 < 2
//             AluEvent::new(0, Opcode::SLT, 0, 3, 2, false),
//             // 1 == 2 < 3
//             AluEvent::new(0, Opcode::SLT, 1, 2, 3, false),
//             // 0 == 5 < -3
//             AluEvent::new(0, Opcode::SLT, 0, 5, NEG_3, false),
//             // 1 == -3 < 5
//             AluEvent::new(0, Opcode::SLT, 1, NEG_3, 5, false),
//             // 0 == -3 < -4
//             AluEvent::new(0, Opcode::SLT, 0, NEG_3, NEG_4, false),
//             // 1 == -4 < -3
//             AluEvent::new(0, Opcode::SLT, 1, NEG_4, NEG_3, false),
//             // 0 == 3 < 3
//             AluEvent::new(0, Opcode::SLT, 0, 3, 3, false),
//             // 0 == -3 < -3
//             AluEvent::new(0, Opcode::SLT, 0, NEG_3, NEG_3, false),
//         ];

//         prove_koalabear_template(shard);
//     }

//     #[test]
//     fn prove_koalabear_sltu() {
//         let mut shard = ExecutionRecord::default();

//         const LARGE: u32 = 0b11111111111111111111111111111101;
//         shard.lt_events = vec![
//             // 0 == 3 < 2
//             AluEvent::new(0, Opcode::SLTU, 0, 3, 2, false),
//             // 1 == 2 < 3
//             AluEvent::new(0, Opcode::SLTU, 1, 2, 3, false),
//             // 0 == LARGE < 5
//             AluEvent::new(0, Opcode::SLTU, 0, LARGE, 5, false),
//             // 1 == 5 < LARGE
//             AluEvent::new(0, Opcode::SLTU, 1, 5, LARGE, false),
//             // 0 == 0 < 0
//             AluEvent::new(0, Opcode::SLTU, 0, 0, 0, false),
//             // 0 == LARGE < LARGE
//             AluEvent::new(0, Opcode::SLTU, 0, LARGE, LARGE, false),
//         ];

//         prove_koalabear_template(shard);
//     }

//     #[test]
//     fn test_malicious_lt() {
//         const NUM_TESTS: usize = 5;

//         for opcode in [Opcode::SLTU, Opcode::SLT] {
//             for _ in 0..NUM_TESTS {
//                 let op_b = thread_rng().gen_range(0..u32::MAX);
//                 let op_c = thread_rng().gen_range(0..u32::MAX);

//                 let correct_op_a = if opcode == Opcode::SLTU {
//                     op_b < op_c
//                 } else {
//                     (op_b as i32) < (op_c as i32)
//                 };

//                 let op_a = !correct_op_a;

//                 let instructions = vec![
//                     Instruction::new(opcode, 5, op_b, op_c, true, true),
//                     Instruction::new(Opcode::ADD, 10, 0, 0, false, false),
//                 ];

//                 let program = Program::new(instructions, 0, 0);
//                 let stdin = SP1Stdin::new();

//                 type P = CpuProver<SP1CoreJaggedConfig, RiscvAir<SP1Field>>;

//                 let malicious_trace_pv_generator = move |prover: &P,
//                                                          record: &mut ExecutionRecord|
//                       -> Vec<(
//                     String,
//                     RowMajorMatrix<Val<SP1CoreJaggedConfig>>,
//                 )> {
//                     let mut malicious_record = record.clone();
//                     malicious_record.cpu_events[0].a = op_a as u32;
//                     if let Some(MemoryRecordEnum::Write(mut write_record)) =
//                         malicious_record.cpu_events[0].a_record
//                     {
//                         write_record.value = op_a as u32;
//                     }
//                     let mut traces = prover.generate_traces(&malicious_record);

//                     let lt_chip_name = chip_name!(LtChip, SP1Field);
//                     for (chip_name, trace) in traces.iter_mut() {
//                         if *chip_name == lt_chip_name {
//                             let first_row = trace.row_mut(0);
//                             let first_row: &mut LtCols<SP1Field> = first_row.borrow_mut();
//                             first_row.a = SP1Field::from_bool(op_a);
//                         }
//                     }

//                     traces
//                 };

//                 let result =
//                     run_malicious_test::<P>(program, stdin,
// Box::new(malicious_trace_pv_generator));                 assert!(result.is_err() &&
// result.unwrap_err().is_constraints_failing());             }
//         }
//     }
// }
