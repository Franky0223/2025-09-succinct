use std::borrow::BorrowMut;

use hashbrown::HashMap;
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};
use slop_algebra::PrimeField32;
use slop_matrix::dense::RowMajorMatrix;
use sp1_core_executor::{
    events::{ByteLookupEvent, ByteRecord, SyscallEvent},
    syscalls::SyscallCode,
    ExecutionRecord, Program, RTypeRecord, HALT_PC,
};
use sp1_hypercube::{air::MachineAir, Word};
use sp1_primitives::consts::u64_to_u16_limbs;

use crate::utils::{next_multiple_of_32, zeroed_f_vec};

use super::{
    columns::{SyscallInstrColumns, NUM_SYSCALL_INSTR_COLS},
    SyscallInstrsChip,
};

impl<F: PrimeField32> MachineAir<F> for SyscallInstrsChip {
    type Record = ExecutionRecord;

    type Program = Program;

    fn name(&self) -> String {
        "SyscallInstrs".to_string()
    }

    fn num_rows(&self, input: &Self::Record) -> Option<usize> {
        let nb_rows = input.syscall_events.len();
        let size_log2 = input.fixed_log2_rows::<F, _>(self);
        let padded_nb_rows = next_multiple_of_32(nb_rows, size_log2);
        Some(padded_nb_rows)
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        output: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        let chunk_size = std::cmp::max((input.syscall_events.len()) / num_cpus::get(), 1);
        let padded_nb_rows = <SyscallInstrsChip as MachineAir<F>>::num_rows(self, input).unwrap();
        let mut values = zeroed_f_vec(padded_nb_rows * NUM_SYSCALL_INSTR_COLS);

        let blu_events = values
            .chunks_mut(chunk_size * NUM_SYSCALL_INSTR_COLS)
            .enumerate()
            .par_bridge()
            .map(|(i, rows)| {
                let mut blu: HashMap<ByteLookupEvent, usize> = HashMap::new();
                rows.chunks_mut(NUM_SYSCALL_INSTR_COLS).enumerate().for_each(|(j, row)| {
                    let idx = i * chunk_size + j;
                    let cols: &mut SyscallInstrColumns<F> = row.borrow_mut();

                    if idx < input.syscall_events.len() {
                        let event = &input.syscall_events[idx];
                        self.event_to_row(&event.0, &event.1, cols, &mut blu);
                        cols.state.populate(&mut blu, event.0.clk, event.0.pc);
                        cols.adapter.populate(&mut blu, event.1);
                    }
                });
                blu
            })
            .collect::<Vec<_>>();

        output.add_byte_lookup_events_from_maps(blu_events.iter().collect_vec());

        // Convert the trace to a row major matrix.
        RowMajorMatrix::new(values, NUM_SYSCALL_INSTR_COLS)
    }

    fn included(&self, shard: &Self::Record) -> bool {
        if let Some(shape) = shard.shape.as_ref() {
            shape.included::<F, _>(self)
        } else {
            !shard.syscall_events.is_empty()
        }
    }
}

impl SyscallInstrsChip {
    fn event_to_row<F: PrimeField32>(
        &self,
        event: &SyscallEvent,
        record: &RTypeRecord,
        cols: &mut SyscallInstrColumns<F>,
        blu: &mut impl ByteRecord,
    ) {
        cols.is_real = F::one();

        cols.op_a_value = Word::from(record.a.value());
        cols.a_low_bytes.populate_u16_to_u8_safe(blu, record.a.prev_value());
        blu.add_u16_range_checks(&u64_to_u16_limbs(record.a.value()));
        let a_prev_value = record.a.prev_value().to_le_bytes().map(F::from_canonical_u8);

        let syscall_id = a_prev_value[0];

        cols.is_halt =
            F::from_bool(syscall_id == F::from_canonical_u32(SyscallCode::HALT.syscall_id()));

        if cols.is_halt == F::one() {
            cols.next_pc = [F::from_canonical_u64(HALT_PC), F::zero(), F::zero()];
        } else {
            cols.next_pc = [
                F::from_canonical_u32(((event.pc & 0xFFFF) as u32) + 4),
                F::from_canonical_u32(((event.pc >> 16) & 0xFFFF) as u32),
                F::from_canonical_u32(((event.pc >> 32) & 0xFFFF) as u32),
            ];
        }

        // Populate `is_enter_unconstrained`.
        cols.is_enter_unconstrained.populate_from_field_element(
            syscall_id - F::from_canonical_u32(SyscallCode::ENTER_UNCONSTRAINED.syscall_id()),
        );

        // Populate `is_hint_len`.
        cols.is_hint_len.populate_from_field_element(
            syscall_id - F::from_canonical_u32(SyscallCode::HINT_LEN.syscall_id()),
        );

        // Populate `is_halt`.
        cols.is_halt_check.populate_from_field_element(
            syscall_id - F::from_canonical_u32(SyscallCode::HALT.syscall_id()),
        );

        // Populate `is_commit`.
        cols.is_commit.populate_from_field_element(
            syscall_id - F::from_canonical_u32(SyscallCode::COMMIT.syscall_id()),
        );

        // Populate `is_page_protect`.
        cols.is_page_protect.populate_from_field_element(
            syscall_id - F::from_canonical_u32(SyscallCode::MPROTECT.syscall_id()),
        );

        // Populate `is_commit_deferred_proofs`.
        cols.is_commit_deferred_proofs.populate_from_field_element(
            syscall_id - F::from_canonical_u32(SyscallCode::COMMIT_DEFERRED_PROOFS.syscall_id()),
        );

        // For `COMMIT` or `COMMIT_DEFERRED_PROOFS`, set the index bitmap and digest word.
        if syscall_id == F::from_canonical_u32(SyscallCode::COMMIT.syscall_id())
            || syscall_id == F::from_canonical_u32(SyscallCode::COMMIT_DEFERRED_PROOFS.syscall_id())
        {
            let digest_idx = record.b.value() as usize;
            cols.index_bitmap[digest_idx] = F::one();
        }

        // If the syscall is `COMMIT`, set the expected public values digest and range check.
        if syscall_id == F::from_canonical_u32(SyscallCode::COMMIT.syscall_id()) {
            let digest_bytes = (record.c.value() as u32).to_le_bytes();
            cols.expected_public_values_digest = digest_bytes.map(F::from_canonical_u8);
            blu.add_u8_range_checks(&digest_bytes);
        }

        // Add the SP1Field range check of the operands.
        if cols.is_halt == F::one() {
            cols.op_b_range_check.populate(Word::from(event.arg1), blu);
        }

        if syscall_id == F::from_canonical_u32(SyscallCode::COMMIT_DEFERRED_PROOFS.syscall_id()) {
            cols.op_c_range_check.populate(Word::from(event.arg2), blu);
        }
    }
}
