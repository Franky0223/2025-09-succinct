use crate::{
    adapter::{
        register::i_type::{ITypeReader, ITypeReaderImmutable, ITypeReaderImmutableInput},
        state::{CPUState, CPUStateInput},
    },
    air::{SP1CoreAirBuilder, SP1Operation},
    memory::MemoryAccessCols,
    operations::{AddressOperation, AddressOperationInput},
    utils::{next_multiple_of_32, zeroed_f_vec},
};
use hashbrown::HashMap;
use itertools::Itertools;
use rayon::iter::{ParallelBridge, ParallelIterator};
use slop_air::{Air, AirBuilder, BaseAir};
use slop_algebra::{AbstractField, PrimeField32};
use slop_matrix::{dense::RowMajorMatrix, Matrix};
use sp1_core_executor::{
    events::{ByteLookupEvent, ByteRecord, MemInstrEvent, MemoryAccessPosition},
    ExecutionRecord, Opcode, Program, CLK_INC, PC_INC,
};
use sp1_derive::AlignedBorrow;
use sp1_hypercube::air::MachineAir;
use sp1_primitives::consts::PROT_READ;
use std::{
    borrow::{Borrow, BorrowMut},
    mem::size_of,
};

#[derive(Default)]
pub struct LoadX0Chip;

pub const NUM_LOAD_X0_COLUMNS: usize = size_of::<LoadX0Columns<u8>>();

/// The column layout for memory load instructions with `op_a = x0`.
#[derive(AlignedBorrow, Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct LoadX0Columns<T> {
    /// The current shard, timestamp, program counter of the CPU.
    pub state: CPUState<T>,

    /// The adapter to read program and register information.
    pub adapter: ITypeReader<T>,

    /// Instance of `AddressOperation` to constrain the memory address.
    pub address_operation: AddressOperation<T>,

    /// Memory consistency columns for the memory access.
    pub memory_access: MemoryAccessCols<T>,

    /// The bit decomposition of the offset.
    pub offset_bit: [T; 3],

    /// Whether this is a load byte instruction.
    pub is_lb: T,

    /// Whether this is a load byte unsigned instruction.
    pub is_lbu: T,

    /// Whether this is a load half instruction.
    pub is_lh: T,

    /// Whether this is a load half unsigned instruction.
    pub is_lhu: T,

    /// Whether this is a load word instruction.
    pub is_lw: T,

    /// Whether this is a load word unsigned instruction.
    pub is_lwu: T,

    /// Whether this is a load double word instruction.
    pub is_ld: T,

    /// Whether the page protection is active.
    pub is_page_protect_active: T,
}

impl<F> BaseAir<F> for LoadX0Chip {
    fn width(&self) -> usize {
        NUM_LOAD_X0_COLUMNS
    }
}

impl<F: PrimeField32> MachineAir<F> for LoadX0Chip {
    type Record = ExecutionRecord;

    type Program = Program;

    fn name(&self) -> String {
        "LoadX0".to_string()
    }

    fn num_rows(&self, input: &Self::Record) -> Option<usize> {
        let nb_rows = next_multiple_of_32(
            input.memory_load_x0_events.len(),
            input.fixed_log2_rows::<F, _>(self),
        );
        Some(nb_rows)
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        output: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        let chunk_size = std::cmp::max((input.memory_load_x0_events.len()) / num_cpus::get(), 1);
        let padded_nb_rows = <LoadX0Chip as MachineAir<F>>::num_rows(self, input).unwrap();
        let mut values = zeroed_f_vec(padded_nb_rows * NUM_LOAD_X0_COLUMNS);

        let blu_events = values
            .chunks_mut(chunk_size * NUM_LOAD_X0_COLUMNS)
            .enumerate()
            .par_bridge()
            .map(|(i, rows)| {
                let mut blu: HashMap<ByteLookupEvent, usize> = HashMap::new();
                rows.chunks_mut(NUM_LOAD_X0_COLUMNS).enumerate().for_each(|(j, row)| {
                    let idx = i * chunk_size + j;
                    let cols: &mut LoadX0Columns<F> = row.borrow_mut();

                    if idx < input.memory_load_x0_events.len() {
                        let event = &input.memory_load_x0_events[idx];
                        self.event_to_row(&event.0, cols, &mut blu);
                        cols.is_page_protect_active = F::from_canonical_u32(
                            input.public_values.is_untrusted_programs_enabled,
                        );
                        cols.state.populate(&mut blu, event.0.clk, event.0.pc);
                        cols.adapter.populate(&mut blu, event.1);
                    }
                });
                blu
            })
            .collect::<Vec<_>>();

        output.add_byte_lookup_events_from_maps(blu_events.iter().collect_vec());

        // Convert the trace to a row major matrix.
        RowMajorMatrix::new(values, NUM_LOAD_X0_COLUMNS)
    }

    fn included(&self, shard: &Self::Record) -> bool {
        if let Some(shape) = shard.shape.as_ref() {
            shape.included::<F, _>(self)
        } else {
            !shard.memory_load_x0_events.is_empty()
        }
    }
}

impl LoadX0Chip {
    fn event_to_row<F: PrimeField32>(
        &self,
        event: &MemInstrEvent,
        cols: &mut LoadX0Columns<F>,
        blu: &mut HashMap<ByteLookupEvent, usize>,
    ) {
        // Populate memory accesses for reading from memory.
        cols.memory_access.populate(event.mem_access, blu);

        let memory_addr = cols.address_operation.populate(blu, event.b, event.c);
        let bit0 = (memory_addr & 1) as u16;
        let bit1 = ((memory_addr >> 1) & 1) as u16;
        let bit2 = ((memory_addr >> 2) & 1) as u16;
        cols.offset_bit[0] = F::from_canonical_u16(bit0);
        cols.offset_bit[1] = F::from_canonical_u16(bit1);
        cols.offset_bit[2] = F::from_canonical_u16(bit2);

        cols.is_lb = F::from_bool(event.opcode == Opcode::LB);
        cols.is_lbu = F::from_bool(event.opcode == Opcode::LBU);
        cols.is_lh = F::from_bool(event.opcode == Opcode::LH);
        cols.is_lhu = F::from_bool(event.opcode == Opcode::LHU);
        cols.is_lw = F::from_bool(event.opcode == Opcode::LW);
        cols.is_lwu = F::from_bool(event.opcode == Opcode::LWU);
        cols.is_ld = F::from_bool(event.opcode == Opcode::LD);
    }
}

impl<AB> Air<AB> for LoadX0Chip
where
    AB: SP1CoreAirBuilder,
    AB::Var: Sized,
{
    #[inline(never)]
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let local = main.row_slice(0);
        let local: &LoadX0Columns<AB::Var> = (*local).borrow();

        let clk_high = local.state.clk_high::<AB>();
        let clk_low = local.state.clk_low::<AB>();

        // SAFETY: All selectors `is_lb`, `is_lbu`, `is_lh`, `is_lhu`, `is_lw`, `is_lwu`, `is_ld`
        // are checked to be boolean. Each "real" row has exactly one selector turned on, as
        // `is_real`, the sum of the selectors, is boolean. Therefore, the `opcode` matches the
        // corresponding opcode.
        let opcode = AB::Expr::from_canonical_u32(Opcode::LB as u32) * local.is_lb
            + AB::Expr::from_canonical_u32(Opcode::LBU as u32) * local.is_lbu
            + AB::Expr::from_canonical_u32(Opcode::LH as u32) * local.is_lh
            + AB::Expr::from_canonical_u32(Opcode::LHU as u32) * local.is_lhu
            + AB::Expr::from_canonical_u32(Opcode::LW as u32) * local.is_lw
            + AB::Expr::from_canonical_u32(Opcode::LWU as u32) * local.is_lwu
            + AB::Expr::from_canonical_u32(Opcode::LD as u32) * local.is_ld;

        // Compute instruction field constants
        let funct3 = local.is_lb * AB::Expr::from_canonical_u8(Opcode::LB.funct3().unwrap())
            + local.is_lbu * AB::Expr::from_canonical_u8(Opcode::LBU.funct3().unwrap())
            + local.is_lh * AB::Expr::from_canonical_u8(Opcode::LH.funct3().unwrap())
            + local.is_lhu * AB::Expr::from_canonical_u8(Opcode::LHU.funct3().unwrap())
            + local.is_lw * AB::Expr::from_canonical_u8(Opcode::LW.funct3().unwrap())
            + local.is_lwu * AB::Expr::from_canonical_u8(Opcode::LWU.funct3().unwrap())
            + local.is_ld * AB::Expr::from_canonical_u8(Opcode::LD.funct3().unwrap());
        let funct7 = local.is_lb * AB::Expr::from_canonical_u8(Opcode::LB.funct7().unwrap_or(0))
            + local.is_lbu * AB::Expr::from_canonical_u8(Opcode::LBU.funct7().unwrap_or(0))
            + local.is_lh * AB::Expr::from_canonical_u8(Opcode::LH.funct7().unwrap_or(0))
            + local.is_lhu * AB::Expr::from_canonical_u8(Opcode::LHU.funct7().unwrap_or(0))
            + local.is_lw * AB::Expr::from_canonical_u8(Opcode::LW.funct7().unwrap_or(0))
            + local.is_lwu * AB::Expr::from_canonical_u8(Opcode::LWU.funct7().unwrap_or(0))
            + local.is_ld * AB::Expr::from_canonical_u8(Opcode::LD.funct7().unwrap_or(0));
        let base_opcode = local.is_lb * AB::Expr::from_canonical_u32(Opcode::LB.base_opcode().0)
            + local.is_lbu * AB::Expr::from_canonical_u32(Opcode::LBU.base_opcode().0)
            + local.is_lh * AB::Expr::from_canonical_u32(Opcode::LH.base_opcode().0)
            + local.is_lhu * AB::Expr::from_canonical_u32(Opcode::LHU.base_opcode().0)
            + local.is_lw * AB::Expr::from_canonical_u32(Opcode::LW.base_opcode().0)
            + local.is_lwu * AB::Expr::from_canonical_u32(Opcode::LWU.base_opcode().0)
            + local.is_ld * AB::Expr::from_canonical_u32(Opcode::LD.base_opcode().0);
        let instr_type = local.is_lb
            * AB::Expr::from_canonical_u32(Opcode::LB.instruction_type().0 as u32)
            + local.is_lbu * AB::Expr::from_canonical_u32(Opcode::LBU.instruction_type().0 as u32)
            + local.is_lh * AB::Expr::from_canonical_u32(Opcode::LH.instruction_type().0 as u32)
            + local.is_lhu * AB::Expr::from_canonical_u32(Opcode::LHU.instruction_type().0 as u32)
            + local.is_lw * AB::Expr::from_canonical_u32(Opcode::LW.instruction_type().0 as u32)
            + local.is_lwu * AB::Expr::from_canonical_u32(Opcode::LWU.instruction_type().0 as u32)
            + local.is_ld * AB::Expr::from_canonical_u32(Opcode::LD.instruction_type().0 as u32);
        let is_real = local.is_lb
            + local.is_lbu
            + local.is_lh
            + local.is_lhu
            + local.is_lw
            + local.is_lwu
            + local.is_ld;
        builder.assert_bool(local.is_lb);
        builder.assert_bool(local.is_lbu);
        builder.assert_bool(local.is_lh);
        builder.assert_bool(local.is_lhu);
        builder.assert_bool(local.is_lw);
        builder.assert_bool(local.is_lwu);
        builder.assert_bool(local.is_ld);
        builder.assert_bool(is_real.clone());

        // Step 1. Compute the address, and check offsets and address bounds.
        let aligned_addr = <AddressOperation<AB::F> as SP1Operation<AB>>::eval(
            builder,
            AddressOperationInput::new(
                local.adapter.b().map(Into::into),
                local.adapter.c().map(Into::into),
                local.offset_bit[0].into(),
                local.offset_bit[1].into(),
                local.offset_bit[2].into(),
                is_real.clone(),
                local.address_operation,
            ),
        );

        // Check the alignment of the address.
        builder.when(local.is_ld).assert_zero(local.offset_bit[2]);
        builder.when(local.is_lw + local.is_lwu + local.is_ld).assert_zero(local.offset_bit[1]);
        builder
            .when(local.is_lh + local.is_lhu + local.is_lw + local.is_lwu + local.is_ld)
            .assert_zero(local.offset_bit[0]);

        // Step 2. Read the memory address and check page prot access.
        builder.eval_memory_access_read(
            clk_high.clone(),
            clk_low.clone() + AB::Expr::from_canonical_u32(MemoryAccessPosition::Memory as u32),
            &aligned_addr.clone().map(Into::into),
            local.memory_access,
            is_real.clone(),
        );

        // Check page protect active is set correctly based on public value and is_real
        let public_values = builder.extract_public_values();
        let expected_page_protect_active =
            public_values.is_untrusted_programs_enabled.into() * is_real.clone();
        builder.assert_eq(local.is_page_protect_active, expected_page_protect_active);

        builder.send_page_prot(
            clk_high.clone(),
            clk_low.clone() + AB::Expr::from_canonical_u32(MemoryAccessPosition::Memory as u32),
            &aligned_addr.map(Into::into),
            AB::Expr::from_canonical_u8(PROT_READ),
            local.is_page_protect_active.into(),
        );

        // This chip is specifically for load operations with `op_a = x0`.
        builder.when(is_real.clone()).assert_one(local.adapter.op_a_0);

        // Constrain the state of the CPU.
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
        // Since `op_a = x0`, it's immutable.
        <ITypeReaderImmutable as SP1Operation<AB>>::eval(
            builder,
            ITypeReaderImmutableInput::new(
                clk_high,
                clk_low,
                local.state.pc,
                opcode,
                [instr_type, base_opcode, funct3, funct7],
                local.adapter,
                is_real.clone(),
            ),
        );
    }
}
