use serde::{Deserialize, Serialize};
use slop_air::AirBuilder;
use slop_algebra::{AbstractField, Field, PrimeField32};
use sp1_core_executor::{
    events::{ByteRecord, MemoryAccessPosition},
    RTypeRecord,
};
use sp1_derive::{AlignedBorrow, InputExpr, InputParams, IntoShape, SP1OperationBuilder};

use sp1_hypercube::{air::SP1AirBuilder, Word};

use crate::{
    air::{MemoryAirBuilder, ProgramAirBuilder, SP1Operation, WordAirBuilder},
    memory::RegisterAccessCols,
    program::instruction::InstructionCols,
};

/// A set of columns to read operations with op_a, op_b, op_c being registers.
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
pub struct RTypeReader<T> {
    pub op_a: T,
    pub op_a_memory: RegisterAccessCols<T>,
    pub op_a_0: T,
    pub op_b: T,
    pub op_b_memory: RegisterAccessCols<T>,
    pub op_c: T,
    pub op_c_memory: RegisterAccessCols<T>,
    pub is_trusted: T,
}

impl<F: PrimeField32> RTypeReader<F> {
    pub fn populate(&mut self, blu_events: &mut impl ByteRecord, record: RTypeRecord) {
        self.op_a = F::from_canonical_u8(record.op_a);
        self.op_a_memory.populate(record.a, blu_events);
        self.op_a_0 = F::from_bool(record.op_a == 0);
        self.op_b = F::from_canonical_u64(record.op_b);
        self.op_b_memory.populate(record.b, blu_events);
        self.op_c = F::from_canonical_u64(record.op_c);
        self.op_c_memory.populate(record.c, blu_events);
        self.is_trusted = F::from_bool(!record.is_untrusted);
    }
}

impl<T> RTypeReader<T> {
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

impl<F: Field> RTypeReader<F> {
    #[allow(clippy::too_many_arguments)]
    pub fn eval<AB: SP1AirBuilder + MemoryAirBuilder + ProgramAirBuilder>(
        builder: &mut AB,
        clk_high: AB::Expr,
        clk_low: AB::Expr,
        pc: [AB::Var; 3],
        opcode: impl Into<AB::Expr> + Clone,
        instr_field_consts: [AB::Expr; 4],
        op_a_write_value: Word<impl Into<AB::Expr> + Clone>,
        cols: RTypeReader<AB::Var>,
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

        let instruction = InstructionCols {
            opcode: opcode.clone().into(),
            op_a: cols.op_a.into(),
            op_b: Word::extend_expr::<AB>(cols.op_b.into()),
            op_c: Word::extend_expr::<AB>(cols.op_c.into()),
            op_a_0: cols.op_a_0.into(),
            imm_b: AB::Expr::zero(),
            imm_c: AB::Expr::zero(),
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
        builder.eval_register_access_read(
            clk_high.clone(),
            clk_low.clone() + AB::Expr::from_canonical_u32(MemoryAccessPosition::C as u32),
            [cols.op_c.into(), AB::Expr::zero(), AB::Expr::zero()],
            cols.op_c_memory,
            is_real,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn eval_op_a_immutable<AB: SP1AirBuilder + MemoryAirBuilder + ProgramAirBuilder>(
        builder: &mut AB,
        clk_high: AB::Expr,
        clk_low: AB::Expr,
        pc: [AB::Var; 3],
        opcode: impl Into<AB::Expr> + Clone,
        instr_field_consts: [AB::Expr; 4],
        cols: RTypeReader<AB::Var>,
        is_real: AB::Expr,
    ) {
        Self::eval(
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
pub struct RTypeReaderInput<AB: SP1AirBuilder, T: Into<AB::Expr> + Clone> {
    pub clk_high: AB::Expr,
    pub clk_low: AB::Expr,
    pub pc: [AB::Var; 3],
    pub opcode: AB::Expr,
    pub instr_field_consts: [AB::Expr; 4],
    pub op_a_write_value: Word<T>,
    pub cols: RTypeReader<AB::Var>,
    pub is_real: AB::Expr,
}

impl<AB: SP1AirBuilder> SP1Operation<AB> for RTypeReader<AB::F> {
    type Input = RTypeReaderInput<AB, AB::Expr>;
    type Output = ();

    fn lower(builder: &mut AB, input: Self::Input) -> Self::Output {
        Self::eval(
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

#[derive(Debug, Clone, SP1OperationBuilder)]
pub struct RTypeReaderImmutable;

#[allow(clippy::too_many_arguments)]
#[derive(Debug, Clone, InputParams, InputExpr)]
pub struct RTypeReaderImmutableInput<AB: SP1AirBuilder> {
    pub clk_high: AB::Expr,
    pub clk_low: AB::Expr,
    pub pc: [AB::Var; 3],
    pub opcode: AB::Expr,
    pub instr_field_consts: [AB::Expr; 4],
    pub cols: RTypeReader<AB::Var>,
    pub is_real: AB::Expr,
}

impl<AB: SP1AirBuilder> SP1Operation<AB> for RTypeReaderImmutable {
    type Input = RTypeReaderImmutableInput<AB>;
    type Output = ();

    fn lower(builder: &mut AB, input: Self::Input) -> Self::Output {
        RTypeReader::<AB::F>::eval_op_a_immutable(
            builder,
            input.clk_high,
            input.clk_low,
            input.pc,
            input.opcode,
            input.instr_field_consts,
            input.cols,
            input.is_real,
        )
    }
}

// impl<T: Into<<ConstraintCompiler as AirBuilder>::Expr> + Clone>
//     Into<Shape<ExprRef<<ConstraintCompiler as AirBuilder>::F>,
// ExprExtRef<sp1_hypercube::ir::EF>>>     for RTypeReaderInput<ConstraintCompiler>
// {
//     fn into(
//         self,
//     ) -> Shape<ExprRef<<ConstraintCompiler as AirBuilder>::F>, ExprExtRef<sp1_hypercube::ir::EF>>
// {         Shape::Struct(
//             "RTypeReaderInput".to_string(),
//             vec![
//                 ("clk_high".to_string(), Box::new(self.clk_high.into())),
//                 ("clk_low".to_string(), Box::new(self.clk_low.into())),
//                 ("pc".to_string(), Box::new(self.pc.into())),
//                 ("opcode".to_string(), Box::new(self.opcode.into())),
//                 ("op_a_write_value".to_string(), Box::new(self.op_a_write_value.into())),
//                 ("cols".to_string(), Box::new(self.cols.into())),
//                 ("is_real".to_string(), Box::new(self.is_real.into())),
//             ],
//         )
//     }
// }

// impl RTypeReaderInput<ConstraintCompiler>
// {
//     // fn params_vec(
//     //     self,
//     // ) -> Vec<(
//     //     String,
//     //     Shape<ExprRef<<ConstraintCompiler as AirBuilder>::F>,
// ExprExtRef<sp1_hypercube::ir::EF>>,     // )> {
//     //     vec![
//     //         // for demonstration only; not all fields are filled in
//     //         ("clk_high".to_string(), self.clk_high.into()),
//     //         ("op_a_write_value".to_string(), self.op_a_write_value.into()),
//     //     ]
//     // }
//
//     fn to_input(&self, ctx: &mut FuncCtx) -> RTypeReaderInput<ConstraintCompiler> {
//         RTypeReaderInput::new(
//             Expr::input_arg(ctx),
//             Expr::input_arg(ctx),
//             Expr::input_arg(ctx),
//             Expr::input_arg(ctx),
//             Expr::input_from_struct(ctx),
//             Expr::input_from_struct(ctx),
//             Expr::input_arg(ctx),
//         )
//     }
// }
