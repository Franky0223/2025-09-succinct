use std::collections::HashMap;

use itertools::Itertools;
use slop_algebra::{ExtensionField, Field};

use crate::{
    air::AirInteraction,
    ir::{ExprExtRef, ExprRef, IrVar, Shape},
    InteractionKind,
};

// TODO(gzgz): implement constructor and destructor
impl<F: Field, EF: ExtensionField<F>> Shape<ExprRef<F>, ExprExtRef<EF>> {
    /// Output the string that would construct a value of this [Shape]
    pub fn to_lean_constructor(&self, mapping: &HashMap<usize, String>) -> String {
        match self {
            Shape::Unit => unimplemented!("Unit shouldn't appear in constructors"),
            Shape::Expr(expr) => expr.to_lean_string(mapping),
            Shape::ExprExt(_) => todo!(),
            Shape::Word(word) => {
                format!("#v[{}]", word.iter().map(|x| x.to_lean_string(mapping)).join(", "))
            }
            Shape::Array(vals) => {
                format!("#v[{}]", vals.iter().map(|x| x.to_lean_constructor(mapping)).join(", "))
            }
            Shape::Struct(_, fields) => {
                format!(
                    "{{ {} }}",
                    fields
                        .iter()
                        .map(|(field_name, field_val)| format!(
                            "{field_name} := {}",
                            field_val.to_lean_constructor(mapping)
                        ))
                        .join(", ")
                )
            }
        }
    }

    /// Output the string that would destruct a value of this [Shape]
    pub fn to_lean_destructor(&self) -> String {
        match self {
            Shape::Unit => unimplemented!("Unit shouldn't appear in destructors"),
            Shape::Expr(expr) => expr.to_lean_string(&HashMap::default()),
            Shape::ExprExt(_) => todo!(),
            Shape::Word(word) => format!(
                "⟨⟨[{}]⟩, _⟩",
                word.iter().map(|x| x.to_lean_string(&HashMap::default())).join(", ")
            ),
            Shape::Array(vals) => {
                format!("⟨⟨[{}]⟩, _⟩", vals.iter().map(|x| x.to_lean_destructor()).join(", "))
            }
            Shape::Struct(_, _) => todo!("why would you need to destruct a struct"),
        }
    }

    /// Calculates the full variable name that corresponds to `InputArg(x)`.
    ///
    /// For example,
    /// ```lean
    /// structure AddOperation where
    ///   value : Word SP1Field
    ///
    /// def AddOperation.constraints
    ///   (b : SP1Field)
    ///   (c : SP1Field)
    ///   (cols : AddOperation)
    ///   (is_real : SP1Field) := sorry
    /// ```
    ///
    /// `Expr(InputArg(3))` then maps to "cols.value[1]" because if you recursively flatten the
    /// input arguments to `AddOperation.constraints` in argument/field declaration order, then the
    /// element at index 3 corresponds to `cols.value[1]`.
    pub fn map_input(&self, prefix: String, input_mapping: &mut HashMap<usize, String>) {
        match self {
            Shape::Unit => unimplemented!("Unit shouldn't appear as input"),
            Shape::Expr(ExprRef::IrVar(IrVar::InputArg(idx))) => {
                input_mapping.insert(*idx, prefix);
            }
            Shape::Word(vals) => {
                for (i, val) in vals.iter().enumerate() {
                    match val {
                        ExprRef::IrVar(IrVar::InputArg(idx)) => {
                            // In Mathlib, c[i] means some permutation stuff...
                            if prefix == "c" {
                                input_mapping.insert(*idx, format!("cc[{i}]"));
                            } else {
                                input_mapping.insert(*idx, format!("{prefix}[{i}]"));
                            }
                        }
                        _ => unimplemented!("map_input must be backed by Input(x)"),
                    }
                }
            }
            Shape::Array(vals) => {
                for (i, val) in vals.iter().enumerate() {
                    val.map_input(format!("{prefix}[{i}]"), input_mapping);
                }
            }
            Shape::Struct(_, fields) => {
                for (name, field) in fields {
                    field.map_input(format!("{prefix}.{name}"), input_mapping);
                }
            }
            _ => unimplemented!(),
        }
    }
}

impl<F: Field> AirInteraction<ExprRef<F>> {
    /// Converts an Air interaction to an `AirInteraction` in sp1-lean.
    pub fn to_lean_string(&self, input_mapping: &HashMap<usize, String>) -> String {
        let mut res = "(".to_string();

        let kind_str = match self.kind {
            InteractionKind::Memory => ".memory",
            InteractionKind::Program => ".program",
            InteractionKind::Byte => ".byte",
            InteractionKind::State => ".state",
            _ => todo!(),
        };
        res.push_str(kind_str);

        match self.kind {
            InteractionKind::Byte => {
                assert_eq!(self.values.len(), 4);
                for (idx, val) in self.values.iter().enumerate() {
                    if idx == 0 {
                        // ByteOpcode
                        res.push_str(&format!(
                            " (ByteOpcode.ofNat {})",
                            val.to_lean_string(input_mapping)
                        ));
                    } else {
                        res.push_str(&format!(" {}", val.to_lean_string(input_mapping)));
                    }
                }
            }
            InteractionKind::Memory => {
                assert_eq!(self.values.len(), 9);
                for val in &self.values {
                    res.push_str(&format!(" {}", val.to_lean_string(input_mapping)));
                }
            }
            InteractionKind::State => {
                assert_eq!(self.values.len(), 5);
                for val in &self.values {
                    res.push_str(&format!(" {}", val.to_lean_string(input_mapping)));
                }
            }
            InteractionKind::Program => {
                assert_eq!(self.values.len(), 19);

                for (idx, val) in self.values.iter().enumerate() {
                    if idx == 3 {
                        // Opcode
                        res.push_str(&format!(
                            " (Opcode.ofNat {})",
                            val.to_lean_string(input_mapping)
                        ));
                    } else {
                        res.push_str(&format!(" {}", val.to_lean_string(input_mapping)));
                    }
                }
            }
            _ => {
                todo!();
            }
        }

        res.push(')');
        res
    }
}
