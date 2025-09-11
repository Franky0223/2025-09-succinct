use serde::{Deserialize, Serialize};
use slop_air::AirBuilder;
use slop_algebra::{AbstractField, Field, PrimeField32};
use sp1_core_executor::{
    events::{ByteRecord, MemoryAccessPosition},
    ALUTypeRecord,
};
use sp1_derive::{AlignedBorrow, InputExpr, InputParams, IntoShape, SP1OperationBuilder};

use sp1_hypercube::{air::SP1AirBuilder, Word};

use crate::{
    air::{MemoryAirBuilder, ProgramAirBuilder, SP1Operation, WordAirBuilder},
    memory::RegisterAccessCols,
    program::instruction::InstructionCols,
};

/// A set of columns to read operations with op_a and op_b being registers and op_c being a register
/// or immediate.
#[derive(
    AlignedBorrow,
    Default,
    Debug,
    Clone,
    Copy,
    Serialize,
    Deserialize,
    IntoShape,
    SP1OperationBuilder,
)]
#[repr(C)]
pub struct ALUTypeReader<T> {
    pub op_a: T,
    pub op_a_memory: RegisterAccessCols<T>,
    pub op_a_0: T,
    pub op_b: T,
    pub op_b_memory: RegisterAccessCols<T>,
    pub op_c: Word<T>,
    pub op_c_memory: RegisterAccessCols<T>,
    pub imm_c: T,
    pub is_trusted: T,
}

impl<T> ALUTypeReader<T> {
    pub fn prev_a(&self) -> &Word<T> {
        &self.op_a_memory.prev_value
    }

    pub fn b(&self) -> &Word<T> {
        &self.op_b_memory.prev_value
    }

    pub fn c(&self) -> &Word<T> {
        &self.op_c_memory.prev_value
    }
}

impl<F: PrimeField32> ALUTypeReader<F> {
    pub fn populate(&mut self, blu_events: &mut impl ByteRecord, record: ALUTypeRecord) {
        self.op_a = F::from_canonical_u8(record.op_a);
        self.op_a_memory.populate(record.a, blu_events);
        self.op_a_0 = F::from_bool(record.op_a == 0);
        self.op_b = F::from_canonical_u64(record.op_b);
        self.op_b_memory.populate(record.b, blu_events);
        self.op_c = Word::from(record.op_c);
        let imm_c = record.c.is_none();
        self.imm_c = F::from_bool(imm_c);
        if imm_c {
            self.op_c_memory.prev_value = self.op_c;
        } else {
            self.op_c_memory.populate(record.c.unwrap(), blu_events);
        }
        self.is_trusted = F::from_bool(!record.is_untrusted);
    }
}

impl<F: Field> ALUTypeReader<F> {
    #[allow(clippy::too_many_arguments)]
    fn eval_alu_reader<AB: SP1AirBuilder + MemoryAirBuilder + ProgramAirBuilder>(
        builder: &mut AB,
        clk_high: AB::Expr,
        clk_low: AB::Expr,
        pc: [AB::Var; 3],
        opcode: impl Into<AB::Expr> + Clone,
        instr_field_consts: [AB::Expr; 4],
        op_a_write_value: Word<impl Into<AB::Expr> + Clone>,
        cols: ALUTypeReader<AB::Var>,
        is_real: AB::Expr,
    ) {
        builder.assert_bool(is_real.clone());
        let is_untrusted = is_real.clone() - cols.is_trusted;
        builder.assert_bool(is_untrusted.clone());
        builder.assert_bool(cols.is_trusted);

        // A real row must be executing either a trusted program or untrusted program.
        builder.assert_eq(is_untrusted.clone() + cols.is_trusted, is_real.clone());

        // If the row is running an untrusted program, the page protection checks must be on.
        let public_values = builder.extract_public_values();
        builder.when(is_untrusted.clone()).assert_one(public_values.is_untrusted_programs_enabled);

        // Assert that `imm_c` is zero if the operation is not real.
        // This is to ensure that the `op_c` read multiplicity is zero on padding rows.
        builder.when_not(is_real.clone()).assert_eq(cols.imm_c, AB::Expr::zero());

        let instruction: InstructionCols<AB::Expr> = InstructionCols {
            opcode: opcode.clone().into(),
            op_a: cols.op_a.into(),
            op_b: Word::extend_expr::<AB>(cols.op_b.into()),
            op_c: cols.op_c.map(Into::into),
            op_a_0: cols.op_a_0.into(),
            imm_b: AB::Expr::zero(),
            imm_c: cols.imm_c.into(),
        };

        builder.send_program(pc, instruction.clone(), cols.is_trusted);
        builder.send_instruction_fetch(
            pc,
            instruction,
            instr_field_consts,
            [clk_high.clone(), clk_low.clone()],
            is_untrusted.clone(),
        );

        // Assert that `op_a` is zero if `op_a_0` is true.
        builder.when(cols.op_a_0).assert_word_eq(op_a_write_value.clone(), Word::zero::<AB>());
        builder.eval_register_access_write(
            clk_high.clone(),
            clk_low.clone() + AB::Expr::from_canonical_u32(MemoryAccessPosition::A as u32),
            [cols.op_a.into(), AB::Expr::zero(), AB::Expr::zero()],
            cols.op_a_memory,
            op_a_write_value,
            is_real.clone(),
        );
        builder.eval_register_access_read(
            clk_high.clone(),
            clk_low.clone() + AB::Expr::from_canonical_u32(MemoryAccessPosition::B as u32),
            [cols.op_b.into(), AB::Expr::zero(), AB::Expr::zero()],
            cols.op_b_memory,
            is_real.clone(),
        );
        // Read the `op_c[0]` register only when `imm_c` is zero and `is_real` is true.
        builder.eval_register_access_read(
            clk_high.clone(),
            clk_low.clone() + AB::Expr::from_canonical_u32(MemoryAccessPosition::C as u32),
            [cols.op_c[0].into(), AB::Expr::zero(), AB::Expr::zero()],
            cols.op_c_memory,
            is_real - cols.imm_c,
        );
        // If `op_c` is an immediate, assert that `op_c` value is copied into
        // `op_c_memory.prev_value`.
        builder.when(cols.imm_c).assert_word_eq(cols.op_c_memory.prev_value, cols.op_c);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn eval_op_a_immutable<AB: SP1AirBuilder + MemoryAirBuilder + ProgramAirBuilder>(
        builder: &mut AB,
        clk_high: AB::Expr,
        clk_low: AB::Expr,
        pc: [AB::Var; 3],
        opcode: impl Into<AB::Expr> + Clone,
        instr_field_consts: [AB::Expr; 4],
        cols: ALUTypeReader<AB::Var>,
        is_real: AB::Expr,
    ) {
        Self::eval_alu_reader(
            builder,
            clk_high,
            clk_low,
            pc,
            opcode,
            instr_field_consts,
            cols.op_a_memory.prev_value,
            cols,
            is_real,
        );
    }
}

#[derive(Clone, InputParams, InputExpr)]
pub struct ALUTypeReaderInput<AB: SP1AirBuilder, T: Into<AB::Expr> + Clone> {
    pub clk_high: AB::Expr,
    pub clk_low: AB::Expr,
    pub pc: [AB::Var; 3],
    pub opcode: AB::Expr,
    pub instr_field_consts: [AB::Expr; 4],
    pub op_a_write_value: Word<T>,
    pub cols: ALUTypeReader<AB::Var>,
    pub is_real: AB::Expr,
}

impl<AB: SP1AirBuilder> SP1Operation<AB> for ALUTypeReader<AB::F> {
    type Input = ALUTypeReaderInput<AB, AB::Expr>;
    type Output = ();

    fn lower(builder: &mut AB, input: Self::Input) -> Self::Output {
        Self::eval_alu_reader(
            builder,
            input.clk_high,
            input.clk_low,
            input.pc,
            input.opcode,
            input.instr_field_consts,
            input.op_a_write_value,
            input.cols,
            input.is_real,
        )
    }
}
