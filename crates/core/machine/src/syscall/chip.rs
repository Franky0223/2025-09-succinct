use crate::{air::WordAirBuilder, utils::next_multiple_of_32};
use core::fmt;
use itertools::Itertools;
use slop_air::{Air, BaseAir};
use slop_algebra::{AbstractField, PrimeField32};
use slop_matrix::{dense::RowMajorMatrix, Matrix};
use slop_maybe_rayon::prelude::{IntoParallelRefIterator, ParallelBridge, ParallelIterator};
use sp1_core_executor::{
    events::{ByteRecord, GlobalInteractionEvent, SyscallEvent},
    ExecutionRecord, Program,
};
use sp1_derive::AlignedBorrow;
use sp1_hypercube::{
    air::{AirInteraction, InteractionScope, MachineAir, SP1AirBuilder},
    InteractionKind,
};
use std::{
    borrow::{Borrow, BorrowMut},
    mem::size_of,
};
/// The number of main trace columns for `SyscallChip`.
pub const NUM_SYSCALL_COLS: usize = size_of::<SyscallCols<u8>>();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SyscallShardKind {
    Core,
    Precompile,
}

/// A chip that stores the syscall invocations.
pub struct SyscallChip {
    shard_kind: SyscallShardKind,
}

impl SyscallChip {
    pub const fn new(shard_kind: SyscallShardKind) -> Self {
        Self { shard_kind }
    }

    pub const fn core() -> Self {
        Self::new(SyscallShardKind::Core)
    }

    pub const fn precompile() -> Self {
        Self::new(SyscallShardKind::Precompile)
    }

    pub fn shard_kind(&self) -> SyscallShardKind {
        self.shard_kind
    }
}

/// The column layout for the chip.
#[derive(AlignedBorrow, Clone, Copy)]
#[repr(C)]
pub struct SyscallCols<T: Copy> {
    /// The high bits of the clk of the syscall.
    pub clk_high: T,

    /// The low bits of clk of the syscall.
    pub clk_low: T,

    /// The syscall_id of the syscall.
    pub syscall_id: T,

    /// The arg1.
    pub arg1: [T; 3],

    /// The arg2.
    pub arg2: [T; 3],

    pub is_real: T,
}

impl<F: PrimeField32> MachineAir<F> for SyscallChip {
    type Record = ExecutionRecord;

    type Program = Program;

    fn name(&self) -> String {
        format!("Syscall{}", self.shard_kind).to_string()
    }

    fn generate_dependencies(&self, input: &ExecutionRecord, output: &mut ExecutionRecord) {
        let events = match self.shard_kind {
            SyscallShardKind::Core => &input
                .syscall_events
                .iter()
                .map(|(event, _)| event)
                .filter(|e| e.should_send)
                .copied()
                .collect::<Vec<_>>(),
            SyscallShardKind::Precompile => &input
                .precompile_events
                .all_events()
                .map(|(event, _)| event.to_owned())
                .collect::<Vec<_>>(),
        };

        let events = events
            .iter()
            .filter(|e| e.should_send)
            .map(|event| {
                let mut blu = Vec::new();
                blu.add_u8_range_checks(&[event.syscall_id as u8]);
                blu.add_u16_range_checks(&[(event.arg1 & 0xFFFF) as u16]);
                let global_event = GlobalInteractionEvent {
                    message: [
                        (event.clk >> 24) as u32,
                        (event.clk & 0xFFFFFF) as u32,
                        event.syscall_id + (1 << 8) * (event.arg1 & 0xFFFF) as u32,
                        ((event.arg1 >> 16) & 0xFFFF) as u32,
                        ((event.arg1 >> 32) & 0xFFFF) as u32,
                        (event.arg2 & 0xFFFF) as u32,
                        ((event.arg2 >> 16) & 0xFFFF) as u32,
                        ((event.arg2 >> 32) & 0xFFFF) as u32,
                    ],
                    is_receive: self.shard_kind == SyscallShardKind::Precompile,
                    kind: InteractionKind::Syscall as u8,
                };
                output.add_byte_lookup_events(blu);
                global_event
            })
            .collect_vec();
        output.global_interaction_events.extend(events);
    }

    fn num_rows(&self, input: &Self::Record) -> Option<usize> {
        let events = match self.shard_kind {
            SyscallShardKind::Core => &input
                .syscall_events
                .iter()
                .map(|(event, _)| event)
                .filter(|e| e.should_send)
                .copied()
                .collect::<Vec<_>>(),
            SyscallShardKind::Precompile => &input
                .precompile_events
                .all_events()
                .map(|(event, _)| event.to_owned())
                .collect::<Vec<_>>(),
        };
        let nb_rows = events.len();
        let size_log2 = input.fixed_log2_rows::<F, _>(self);
        let padded_nb_rows = next_multiple_of_32(nb_rows, size_log2);
        Some(padded_nb_rows)
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        _output: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        let row_fn = |syscall_event: &SyscallEvent, _: bool| {
            let mut row = [F::zero(); NUM_SYSCALL_COLS];
            let cols: &mut SyscallCols<F> = row.as_mut_slice().borrow_mut();

            cols.clk_high = F::from_canonical_u32((syscall_event.clk >> 24) as u32);
            cols.clk_low = F::from_canonical_u32((syscall_event.clk & 0xFFFFFF) as u32);
            cols.syscall_id = F::from_canonical_u32(syscall_event.syscall_code.syscall_id());

            cols.arg1 = [
                F::from_canonical_u64((syscall_event.arg1 & 0xFFFF) as u64),
                F::from_canonical_u64(((syscall_event.arg1 >> 16) & 0xFFFF) as u64),
                F::from_canonical_u64(((syscall_event.arg1 >> 32) & 0xFFFF) as u64),
            ];
            cols.arg2 = [
                F::from_canonical_u64((syscall_event.arg2 & 0xFFFF) as u64),
                F::from_canonical_u64(((syscall_event.arg2 >> 16) & 0xFFFF) as u64),
                F::from_canonical_u64(((syscall_event.arg2 >> 32) & 0xFFFF) as u64),
            ];

            cols.is_real = F::one();
            row
        };

        let mut rows = match self.shard_kind {
            SyscallShardKind::Core => input
                .syscall_events
                .par_iter()
                .map(|(event, _)| event)
                .filter(|e| e.should_send)
                .map(|event| row_fn(event, false))
                .collect::<Vec<_>>(),
            SyscallShardKind::Precompile => input
                .precompile_events
                .all_events()
                .map(|(event, _)| event)
                .par_bridge()
                .map(|event| row_fn(event, true))
                .collect::<Vec<_>>(),
        };

        // Pad the trace to a power of two depending on the proof shape in `input`.
        rows.resize(
            <SyscallChip as MachineAir<F>>::num_rows(self, input).unwrap(),
            [F::zero(); NUM_SYSCALL_COLS],
        );

        RowMajorMatrix::new(rows.into_iter().flatten().collect::<Vec<_>>(), NUM_SYSCALL_COLS)
    }

    fn included(&self, shard: &Self::Record) -> bool {
        if let Some(shape) = shard.shape.as_ref() {
            shape.included::<F, _>(self)
        } else {
            match self.shard_kind {
                SyscallShardKind::Core => {
                    shard
                        .syscall_events
                        .iter()
                        .map(|(event, _)| event)
                        .filter(|e| e.should_send)
                        .take(1)
                        .count()
                        > 0
                }
                SyscallShardKind::Precompile => {
                    !shard.precompile_events.is_empty()
                        && !shard.contains_cpu()
                        && shard.global_memory_initialize_events.is_empty()
                        && shard.global_memory_finalize_events.is_empty()
                        && shard.global_page_prot_initialize_events.is_empty()
                        && shard.global_page_prot_finalize_events.is_empty()
                }
            }
        }
    }
}

impl<AB> Air<AB> for SyscallChip
where
    AB: SP1AirBuilder,
{
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let local = main.row_slice(0);
        let local: &SyscallCols<AB::Var> = (*local).borrow();

        // Constrain that `local.is_real` is boolean.
        builder.assert_bool(local.is_real);

        builder.assert_eq(
            local.is_real * local.is_real * local.is_real,
            local.is_real * local.is_real * local.is_real,
        );

        // Constrain that the syscall id is 8 bits.
        builder.slice_range_check_u8(&[local.syscall_id], local.is_real);
        // Constrain that the arg1 is 16 bits.
        builder.slice_range_check_u16(&[local.arg1[0]], local.is_real);

        match self.shard_kind {
            SyscallShardKind::Core => {
                builder.receive_syscall(
                    local.clk_high,
                    local.clk_low,
                    local.syscall_id,
                    local.arg1.map(Into::into),
                    local.arg2.map(Into::into),
                    local.is_real,
                    InteractionScope::Local,
                );

                // Send the "send interaction" to the global table.
                builder.send(
                    AirInteraction::new(
                        vec![
                            local.clk_high.into(),
                            local.clk_low.into(),
                            local.syscall_id + local.arg1[0] * AB::F::from_canonical_u32(1 << 8),
                            local.arg1[1].into(),
                            local.arg1[2].into(),
                            local.arg2[0].into(),
                            local.arg2[1].into(),
                            local.arg2[2].into(),
                            AB::Expr::one(),
                            AB::Expr::zero(),
                            AB::Expr::from_canonical_u8(InteractionKind::Syscall as u8),
                        ],
                        local.is_real.into(),
                        InteractionKind::Global,
                    ),
                    InteractionScope::Local,
                );
            }
            SyscallShardKind::Precompile => {
                builder.send_syscall(
                    local.clk_high,
                    local.clk_low,
                    local.syscall_id,
                    local.arg1.map(Into::into),
                    local.arg2.map(Into::into),
                    local.is_real,
                    InteractionScope::Local,
                );

                // Send the "receive interaction" to the global table.
                builder.send(
                    AirInteraction::new(
                        vec![
                            local.clk_high.into(),
                            local.clk_low.into(),
                            local.syscall_id + local.arg1[0] * AB::F::from_canonical_u32(1 << 8),
                            local.arg1[1].into(),
                            local.arg1[2].into(),
                            local.arg2[0].into(),
                            local.arg2[1].into(),
                            local.arg2[2].into(),
                            AB::Expr::zero(),
                            AB::Expr::one(),
                            AB::Expr::from_canonical_u8(InteractionKind::Syscall as u8),
                        ],
                        local.is_real.into(),
                        InteractionKind::Global,
                    ),
                    InteractionScope::Local,
                );
            }
        }
    }
}

impl<F> BaseAir<F> for SyscallChip {
    fn width(&self) -> usize {
        NUM_SYSCALL_COLS
    }
}

impl fmt::Display for SyscallShardKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyscallShardKind::Core => write!(f, "Core"),
            SyscallShardKind::Precompile => write!(f, "Precompile"),
        }
    }
}
