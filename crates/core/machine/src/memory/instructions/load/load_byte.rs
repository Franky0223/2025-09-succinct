use slop_air::{Air, AirBuilder, BaseAir};
use slop_matrix::Matrix;
use sp1_derive::AlignedBorrow;
use sp1_hypercube::{air::BaseAirBuilder, Word};
use std::{
    borrow::{Borrow, BorrowMut},
    mem::size_of,
};

use crate::{
    adapter::{
        register::i_type::{ITypeReader, ITypeReaderInput},
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
use slop_algebra::{AbstractField, Field, PrimeField32};
use slop_matrix::dense::RowMajorMatrix;
use sp1_core_executor::{
    events::{ByteLookupEvent, ByteRecord, MemInstrEvent, MemoryAccessPosition},
    ByteOpcode, ExecutionRecord, Opcode, Program, CLK_INC, PC_INC,
};
use sp1_hypercube::air::MachineAir;
use sp1_primitives::consts::{u64_to_u16_limbs, PROT_READ};

#[derive(Default)]
pub struct LoadByteChip;

pub const NUM_LOAD_BYTE_COLUMNS: usize = size_of::<LoadByteColumns<u8>>();

/// The column layout for memory load byte instructions.
#[derive(AlignedBorrow, Default, Debug, Clone, Copy)]
#[repr(C)]
pub struct LoadByteColumns<T> {
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

    /// The selected limb value.
    pub selected_limb: T,

    /// The lower byte of the selected limb.
    pub selected_limb_low_byte: T,

    /// The selected byte value.
    pub selected_byte: T,

    /// The `MSB` of the byte, if the opcode is `LB`.
    pub msb: T,

    /// Whether this is a load byte instruction.
    pub is_lb: T,

    /// Whether this is a load byte unsigned instruction.
    pub is_lbu: T,

    /// Whether the page protection is active.
    pub is_page_protect_active: T,
}

impl<F> BaseAir<F> for LoadByteChip {
    fn width(&self) -> usize {
        NUM_LOAD_BYTE_COLUMNS
    }
}

impl<F: PrimeField32> MachineAir<F> for LoadByteChip {
    type Record = ExecutionRecord;

    type Program = Program;

    fn name(&self) -> String {
        "LoadByte".to_string()
    }

    fn num_rows(&self, input: &Self::Record) -> Option<usize> {
        let nb_rows = next_multiple_of_32(
            input.memory_load_byte_events.len(),
            input.fixed_log2_rows::<F, _>(self),
        );
        Some(nb_rows)
    }

    fn generate_trace(
        &self,
        input: &ExecutionRecord,
        output: &mut ExecutionRecord,
    ) -> RowMajorMatrix<F> {
        let chunk_size = std::cmp::max((input.memory_load_byte_events.len()) / num_cpus::get(), 1);
        let padded_nb_rows = <LoadByteChip as MachineAir<F>>::num_rows(self, input).unwrap();
        let mut values = zeroed_f_vec(padded_nb_rows * NUM_LOAD_BYTE_COLUMNS);

        let blu_events = values
            .chunks_mut(chunk_size * NUM_LOAD_BYTE_COLUMNS)
            .enumerate()
            .par_bridge()
            .map(|(i, rows)| {
                let mut blu: HashMap<ByteLookupEvent, usize> = HashMap::new();
                rows.chunks_mut(NUM_LOAD_BYTE_COLUMNS).enumerate().for_each(|(j, row)| {
                    let idx = i * chunk_size + j;
                    let cols: &mut LoadByteColumns<F> = row.borrow_mut();

                    if idx < input.memory_load_byte_events.len() {
                        let event = &input.memory_load_byte_events[idx];
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
        RowMajorMatrix::new(values, NUM_LOAD_BYTE_COLUMNS)
    }

    fn included(&self, shard: &Self::Record) -> bool {
        if let Some(shape) = shard.shape.as_ref() {
            shape.included::<F, _>(self)
        } else {
            !shard.memory_load_byte_events.is_empty()
        }
    }
}

impl LoadByteChip {
    fn event_to_row<F: PrimeField32>(
        &self,
        event: &MemInstrEvent,
        cols: &mut LoadByteColumns<F>,
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

        let limb_number = 2 * bit2 + bit1;

        let limb = u64_to_u16_limbs(event.mem_access.value())[limb_number as usize];
        cols.selected_limb = F::from_canonical_u16(limb);
        cols.selected_limb_low_byte = F::from_canonical_u16(limb & 0xFF);
        let byte = limb.to_le_bytes()[bit0 as usize];
        cols.selected_byte = F::from_canonical_u8(byte);
        blu.add_u8_range_checks(&limb.to_le_bytes());

        if event.opcode == Opcode::LB {
            cols.is_lb = F::one();
            cols.msb = F::from_canonical_u8(byte >> 7);
            blu.add_byte_lookup_event(ByteLookupEvent {
                opcode: ByteOpcode::MSB,
                a: (byte >> 7) as u16,
                b: byte,
                c: 0,
            });
        } else {
            cols.is_lbu = F::one();
        }
    }
}

impl<AB> Air<AB> for LoadByteChip
where
    AB: SP1CoreAirBuilder,
    AB::Var: Sized,
{
    #[inline(never)]
    fn eval(&self, builder: &mut AB) {
        let main = builder.main();
        let local = main.row_slice(0);
        let local: &LoadByteColumns<AB::Var> = (*local).borrow();

        let clk_high = local.state.clk_high::<AB>();
        let clk_low = local.state.clk_low::<AB>();

        // SAFETY: All selectors `is_lb`, `is_lbu` are checked to be boolean.
        // Each "real" row has exactly one selector turned on, as `is_real`, the sum of the
        // selectors, is boolean. Therefore, the `opcode` matches the corresponding opcode.
        let opcode = AB::Expr::from_canonical_u32(Opcode::LB as u32) * local.is_lb
            + AB::Expr::from_canonical_u32(Opcode::LBU as u32) * local.is_lbu;

        // Compute instruction field constants
        let funct3 = local.is_lb * AB::Expr::from_canonical_u8(Opcode::LB.funct3().unwrap())
            + local.is_lbu * AB::Expr::from_canonical_u8(Opcode::LBU.funct3().unwrap());
        let funct7 = local.is_lb * AB::Expr::from_canonical_u8(Opcode::LB.funct7().unwrap_or(0))
            + local.is_lbu * AB::Expr::from_canonical_u8(Opcode::LBU.funct7().unwrap_or(0));
        let base_opcode = local.is_lb * AB::Expr::from_canonical_u32(Opcode::LB.base_opcode().0)
            + local.is_lbu * AB::Expr::from_canonical_u32(Opcode::LBU.base_opcode().0);
        let instr_type = local.is_lb
            * AB::Expr::from_canonical_u32(Opcode::LB.instruction_type().0 as u32)
            + local.is_lbu * AB::Expr::from_canonical_u32(Opcode::LBU.instruction_type().0 as u32);
        let is_real = local.is_lb + local.is_lbu;
        builder.assert_bool(local.is_lb);
        builder.assert_bool(local.is_lbu);
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

        // This chip requires `op_a != x0`.
        builder.assert_zero(local.adapter.op_a_0);

        // Step 3. Use the memory value to compute the write value for `op_a`.
        // Select the u16 limb corresponding to the offset.
        builder
            .when_not(local.offset_bit[1])
            .when_not(local.offset_bit[2])
            .assert_eq(local.selected_limb, local.memory_access.prev_value[0]);
        builder
            .when(local.offset_bit[1])
            .when_not(local.offset_bit[2])
            .assert_eq(local.selected_limb, local.memory_access.prev_value[1]);
        builder
            .when_not(local.offset_bit[1])
            .when(local.offset_bit[2])
            .assert_eq(local.selected_limb, local.memory_access.prev_value[2]);
        builder
            .when(local.offset_bit[1])
            .when(local.offset_bit[2])
            .assert_eq(local.selected_limb, local.memory_access.prev_value[3]);

        // Split the u16 limb into two bytes.
        let byte0 = local.selected_limb_low_byte;
        let byte1 = (local.selected_limb - byte0) * AB::F::from_canonical_u32(1 << 8).inverse();
        builder.slice_range_check_u8(&[byte0.into(), byte1.clone()], is_real.clone());
        // Select the u8 byte corresponding to the offset.
        builder.assert_eq(
            local.selected_byte,
            local.offset_bit[0] * byte1 + (AB::Expr::one() - local.offset_bit[0]) * byte0,
        );
        // Get the MSB of the selected byte if the opcode is `LB`.
        // If the opcode is `LBU`, the MSB is constrained to be zero.
        builder.when(local.is_lbu).assert_zero(local.msb);
        builder.send_byte(
            AB::Expr::from_canonical_u32(ByteOpcode::MSB as u32),
            local.msb,
            local.selected_byte,
            AB::Expr::zero(),
            local.is_lb,
        );

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

        // Compute the four limbs of the word to be written to `op_a`.
        let limb0 =
            local.selected_byte + AB::Expr::from_canonical_u32((1 << 16) - (1 << 8)) * local.msb;
        let limb1 = AB::Expr::from_canonical_u32((1 << 16) - 1) * local.msb;
        let limb2 = AB::Expr::from_canonical_u32((1 << 16) - 1) * local.msb;
        let limb3 = AB::Expr::from_canonical_u32((1 << 16) - 1) * local.msb;

        // Constrain the program and register reads.
        <ITypeReader<AB::F> as SP1Operation<AB>>::eval(
            builder,
            ITypeReaderInput::new(
                clk_high,
                clk_low,
                local.state.pc,
                opcode,
                [instr_type, base_opcode, funct3, funct7],
                Word([limb0, limb1, limb2, limb3]),
                local.adapter,
                is_real.clone(),
            ),
        );
    }
}
