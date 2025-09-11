use std::marker::PhantomData;

use super::PublicValuesOutputDigest;
use crate::{
    basefold::{RecursiveBasefoldConfigImpl, RecursiveBasefoldProof, RecursiveBasefoldVerifier},
    challenger::DuplexChallengerVariable,
    jagged::RecursiveJaggedConfig,
    machine::{SP1CompressWithVKeyVerifier, SP1CompressWithVKeyWitnessVariable},
    shard::RecursiveShardVerifier,
    zerocheck::RecursiveVerifierConstraintFolder,
    CircuitConfig, SP1FieldConfigVariable,
};
use slop_air::Air;
use slop_algebra::AbstractField;
use sp1_hypercube::air::MachineAir;
use sp1_primitives::{SP1ExtensionField, SP1Field, SP1GlobalContext};
use sp1_recursion_compiler::ir::{Builder, Felt};
use sp1_recursion_executor::DIGEST_SIZE;

/// A program to verify a single recursive proof representing a complete proof of program execution.
///
/// The root verifier is simply a `SP1CompressVerifier` with an assertion that the `is_complete`
/// flag is set to true.
#[derive(Debug, Clone, Copy)]
pub struct SP1CompressRootVerifierWithVKey<GC, C, SC, A, JC> {
    _phantom: PhantomData<(GC, C, SC, A, JC)>,
}

impl<GC, C, SC, A, JC> SP1CompressRootVerifierWithVKey<GC, C, SC, A, JC>
where
    SC: SP1FieldConfigVariable<
            C,
            FriChallengerVariable = DuplexChallengerVariable<C>,
            DigestVariable = [Felt<SP1Field>; DIGEST_SIZE],
        > + Send
        + Sync,
    C: CircuitConfig<Bit = Felt<SP1Field>>,
    A: MachineAir<SP1Field> + for<'a> Air<RecursiveVerifierConstraintFolder<'a>>,
    JC: RecursiveJaggedConfig<
        F = SP1Field,
        EF = SP1ExtensionField,
        Circuit = C,
        Commitment = SC::DigestVariable,
        Challenger = SC::FriChallengerVariable,
        BatchPcsProof = RecursiveBasefoldProof<RecursiveBasefoldConfigImpl<C, SC>>,
        BatchPcsVerifier = RecursiveBasefoldVerifier<RecursiveBasefoldConfigImpl<C, SC>>,
    >,
{
    pub fn verify(
        builder: &mut Builder<C>,
        machine: &RecursiveShardVerifier<SP1GlobalContext, A, SC, C, JC>,
        input: SP1CompressWithVKeyWitnessVariable<C, SC, JC>,
        value_assertions: bool,
        kind: PublicValuesOutputDigest,
    ) {
        // Assert that the program is complete.
        builder.assert_felt_eq(input.compress_var.is_complete, SP1Field::one());
        // Verify the proof, as a compress proof.
        SP1CompressWithVKeyVerifier::verify(builder, machine, input, value_assertions, kind);
    }
}
