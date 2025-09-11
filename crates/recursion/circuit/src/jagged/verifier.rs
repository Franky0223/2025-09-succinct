use std::marker::PhantomData;

use slop_algebra::{extension::BinomialExtensionField, AbstractField};
use slop_jagged::{
    JaggedBasefoldConfig, JaggedLittlePolynomialVerifierParams, JaggedSumcheckEvalProof,
};
use slop_multilinear::{Evaluations, Mle, Point};
use slop_sumcheck::PartialSumcheckProof;
use sp1_primitives::{SP1ExtensionField, SP1Field};
use sp1_recursion_compiler::{
    circuit::CircuitV2Builder,
    ir::{Builder, Ext, Felt, SymbolicExt},
};

use crate::{
    basefold::{
        stacked::{RecursiveStackedPcsProof, RecursiveStackedPcsVerifier},
        RecursiveBasefoldConfigImpl, RecursiveBasefoldProof, RecursiveBasefoldVerifier,
        RecursiveMultilinearPcsVerifier,
    },
    challenger::FieldChallengerVariable,
    sumcheck::{evaluate_mle_ext, verify_sumcheck},
    AsRecursive, CircuitConfig, SP1FieldConfigVariable,
};

use super::jagged_eval::{RecursiveJaggedEvalConfig, RecursiveJaggedEvalSumcheckConfig};

pub trait RecursiveJaggedConfig: Sized {
    type F;
    type EF: AbstractField;
    type Bit;
    type Circuit: CircuitConfig<Bit = Self::Bit>;
    type Commitment;
    type Challenger: FieldChallengerVariable<Self::Circuit, Self::Bit>;
    type BatchPcsProof;
    type BatchPcsVerifier;
}

pub struct RecursiveJaggedConfigImpl<C, SC, P> {
    _marker: PhantomData<(C, SC, P)>,
}

impl<C: CircuitConfig, SC: SP1FieldConfigVariable<C>, P: RecursiveMultilinearPcsVerifier>
    RecursiveJaggedConfig for RecursiveJaggedConfigImpl<C, SC, P>
{
    type F = SP1Field;
    type EF = SP1ExtensionField;
    type Bit = C::Bit;
    type Circuit = C;
    type Commitment = SC::DigestVariable;
    type Challenger = SC::FriChallengerVariable;
    type BatchPcsProof = RecursiveBasefoldProof<RecursiveBasefoldConfigImpl<C, SC>>;
    type BatchPcsVerifier = RecursiveBasefoldVerifier<RecursiveBasefoldConfigImpl<C, SC>>;
}

pub struct JaggedPcsProofVariable<JC: RecursiveJaggedConfig> {
    pub params: JaggedLittlePolynomialVerifierParams<Felt<JC::F>>,
    pub sumcheck_proof: PartialSumcheckProof<Ext<JC::F, JC::EF>>,
    pub jagged_eval_proof: JaggedSumcheckEvalProof<Ext<SP1Field, SP1ExtensionField>>,
    pub stacked_pcs_proof: RecursiveStackedPcsProof<JC::BatchPcsProof, JC::F, JC::EF>,
    pub added_columns: Vec<usize>,
}

impl<C: CircuitConfig, GC, BC> AsRecursive<C> for JaggedBasefoldConfig<GC, BC>
where
    Self: SP1FieldConfigVariable<C>,
{
    type Recursive = RecursiveJaggedConfigImpl<
        C,
        Self,
        RecursiveBasefoldVerifier<RecursiveBasefoldConfigImpl<C, Self>>,
    >;
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct RecursiveJaggedPcsVerifier<
    SC: SP1FieldConfigVariable<C>,
    C: CircuitConfig,
    JC: RecursiveJaggedConfig<
        BatchPcsVerifier = RecursiveBasefoldVerifier<RecursiveBasefoldConfigImpl<C, SC>>,
    >,
> {
    pub stacked_pcs_verifier: RecursiveStackedPcsVerifier<JC::BatchPcsVerifier>,
    pub max_log_row_count: usize,
    pub jagged_evaluator: RecursiveJaggedEvalSumcheckConfig<SC>,
}

impl<
        SC: SP1FieldConfigVariable<C>,
        C: CircuitConfig,
        JC: RecursiveJaggedConfig<
            F = SP1Field,
            EF = SP1ExtensionField,
            Circuit = C,
            Commitment = SC::DigestVariable,
            Challenger = SC::FriChallengerVariable,
            BatchPcsProof = RecursiveBasefoldProof<RecursiveBasefoldConfigImpl<C, SC>>,
            BatchPcsVerifier = RecursiveBasefoldVerifier<RecursiveBasefoldConfigImpl<C, SC>>,
        >,
    > RecursiveJaggedPcsVerifier<SC, C, JC>
{
    #[allow(dead_code)]
    #[allow(clippy::too_many_arguments)]
    pub fn verify_trusted_evaluations(
        &self,
        builder: &mut Builder<JC::Circuit>,
        commitments: &[JC::Commitment],
        point: Point<Ext<JC::F, JC::EF>>,
        evaluation_claims: &[Evaluations<Ext<JC::F, JC::EF>>],
        proof: &JaggedPcsProofVariable<JC>,
        insertion_points: &[usize],
        challenger: &mut JC::Challenger,
    ) -> Vec<Felt<JC::F>> {
        let JaggedPcsProofVariable {
            stacked_pcs_proof,
            sumcheck_proof,
            jagged_eval_proof,
            params,
            added_columns,
        } = proof;
        let num_col_variables = (params.col_prefix_sums.len() - 1).next_power_of_two().ilog2();
        let z_col =
            (0..num_col_variables).map(|_| challenger.sample_ext(builder)).collect::<Point<_>>();

        let z_row = point;

        // Collect the claims for the different polynomials.
        let mut column_claims =
            evaluation_claims.iter().flatten().flatten().copied().collect::<Vec<_>>();

        // For each commit, Rizz needed a commitment to a vector of length a multiple of
        // 1 << self.pcs.log_stacking_height, and this is achieved by adding a single column of
        // zeroes as the last matrix of the commitment. We insert these "artificial" zeroes
        // into the evaluation claims.
        let zero_ext: Ext<JC::F, JC::EF> = builder.constant(JC::EF::zero());
        for (insertion_point, num_added_columns) in
            insertion_points.iter().rev().zip(added_columns.iter().rev())
        {
            for _ in 0..*num_added_columns {
                column_claims.insert(*insertion_point, zero_ext);
            }
        }

        // Pad the column claims to the next power of two.
        column_claims.resize(column_claims.len().next_power_of_two(), zero_ext);

        let column_mle = Mle::from(column_claims);
        let sumcheck_claim: Ext<JC::F, JC::EF> =
            evaluate_mle_ext(builder, column_mle, z_col.clone())[0];

        builder.assert_ext_eq(sumcheck_claim, sumcheck_proof.claimed_sum);

        builder.cycle_tracker_v2_enter("jagged - verify sumcheck");
        verify_sumcheck::<C, SC>(builder, challenger, sumcheck_proof);
        builder.cycle_tracker_v2_exit();

        builder.cycle_tracker_v2_enter("jagged - jagged-eval");
        let (jagged_eval, prefix_sum_felts) = self.jagged_evaluator.jagged_evaluation(
            builder,
            params,
            z_row,
            z_col,
            sumcheck_proof.point_and_eval.0.clone(),
            jagged_eval_proof,
            challenger,
        );
        builder.cycle_tracker_v2_exit();

        // Compute the expected evaluation of the dense trace polynomial.
        let expected_eval: SymbolicExt<SP1Field, BinomialExtensionField<SP1Field, 4>> =
            sumcheck_proof.point_and_eval.1 / jagged_eval;

        // Verify the evaluation proof.
        let evaluation_point = sumcheck_proof.point_and_eval.0.clone();
        self.stacked_pcs_verifier.verify_trusted_evaluation(
            builder,
            commitments,
            &evaluation_point,
            stacked_pcs_proof,
            expected_eval,
            challenger,
        );
        prefix_sum_felts
    }
}

#[allow(dead_code)]
pub struct RecursiveMachineJaggedPcsVerifier<
    'a,
    SC: SP1FieldConfigVariable<C>,
    C: CircuitConfig,
    JC: RecursiveJaggedConfig<
        BatchPcsVerifier = RecursiveBasefoldVerifier<RecursiveBasefoldConfigImpl<C, SC>>,
    >,
> {
    pub jagged_pcs_verifier: &'a RecursiveJaggedPcsVerifier<SC, C, JC>,
    pub column_counts_by_round: Vec<Vec<usize>>,
}

impl<
        'a,
        SC: SP1FieldConfigVariable<C>,
        C: CircuitConfig,
        JC: RecursiveJaggedConfig<
            F = SP1Field,
            EF = SP1ExtensionField,
            Circuit = C,
            Commitment = SC::DigestVariable,
            Challenger = SC::FriChallengerVariable,
            BatchPcsProof = RecursiveBasefoldProof<RecursiveBasefoldConfigImpl<C, SC>>,
            BatchPcsVerifier = RecursiveBasefoldVerifier<RecursiveBasefoldConfigImpl<C, SC>>,
        >,
    > RecursiveMachineJaggedPcsVerifier<'a, SC, C, JC>
{
    #[allow(dead_code)]
    pub fn new(
        jagged_pcs_verifier: &'a RecursiveJaggedPcsVerifier<SC, C, JC>,
        column_counts_by_round: Vec<Vec<usize>>,
    ) -> Self {
        Self { jagged_pcs_verifier, column_counts_by_round }
    }

    #[allow(dead_code)]
    pub fn verify_trusted_evaluations(
        &self,
        builder: &mut Builder<JC::Circuit>,
        commitments: &[JC::Commitment],
        point: Point<Ext<JC::F, JC::EF>>,
        evaluation_claims: &[Evaluations<Ext<JC::F, JC::EF>>],
        proof: &JaggedPcsProofVariable<JC>,
        challenger: &mut JC::Challenger,
    ) -> Vec<Felt<JC::F>> {
        let insertion_points = self
            .column_counts_by_round
            .iter()
            .scan(0, |state, y| {
                *state += y.iter().sum::<usize>();
                Some(*state)
            })
            .collect::<Vec<_>>();

        self.jagged_pcs_verifier.verify_trusted_evaluations(
            builder,
            commitments,
            point,
            evaluation_claims,
            proof,
            &insertion_points,
            challenger,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::{marker::PhantomData, sync::Arc};

    use rand::{thread_rng, Rng};
    use slop_algebra::AbstractField;
    use slop_basefold::BasefoldVerifier;
    use slop_challenger::{CanObserve, IopCtx};
    use slop_commit::Rounds;
    use slop_jagged::{JaggedPcsProof, JaggedPcsVerifier, JaggedProver, MachineJaggedPcsVerifier};
    use slop_multilinear::{Evaluations, Mle, PaddedMle, Point};
    use sp1_core_machine::utils::setup_logger;
    use sp1_hypercube::{
        inner_perm, SP1BasefoldConfig, SP1CoreJaggedConfig, SP1CpuJaggedProverComponents,
    };
    use sp1_primitives::{SP1DiffusionMatrix, SP1ExtensionField, SP1Field, SP1GlobalContext};
    use sp1_recursion_compiler::circuit::{AsmBuilder, AsmCompiler, AsmConfig, CircuitV2Builder};
    use sp1_recursion_executor::Runtime;

    use crate::{
        basefold::{
            stacked::RecursiveStackedPcsVerifier, tcs::RecursiveMerkleTreeTcs,
            RecursiveBasefoldConfigImpl, RecursiveBasefoldVerifier,
        },
        challenger::{CanObserveVariable, DuplexChallengerVariable},
        jagged::{
            jagged_eval::RecursiveJaggedEvalSumcheckConfig,
            verifier::{
                RecursiveJaggedConfigImpl, RecursiveJaggedPcsVerifier,
                RecursiveMachineJaggedPcsVerifier,
            },
        },
        witness::Witnessable,
    };

    type SC = SP1CoreJaggedConfig;
    type GC = SP1GlobalContext;
    type F = SP1Field;
    type EF = SP1ExtensionField;
    type C = AsmConfig;
    type Prover = JaggedProver<SP1GlobalContext, SP1CpuJaggedProverComponents>;

    async fn generate_jagged_proof(
        jagged_verifier: &JaggedPcsVerifier<GC, SC>,
        round_mles: Rounds<Vec<PaddedMle<F>>>,
        eval_point: Point<EF>,
    ) -> (JaggedPcsProof<GC, SC>, Rounds<<GC as IopCtx>::Digest>, Rounds<Evaluations<EF>>) {
        let jagged_prover = Prover::from_verifier(jagged_verifier);

        let mut challenger = jagged_verifier.challenger();

        let mut prover_data = Rounds::new();
        let mut commitments = Rounds::new();
        for round in round_mles.iter() {
            let (commit, data) =
                jagged_prover.commit_multilinears(round.clone()).await.ok().unwrap();
            challenger.observe(commit);
            let data_bytes = bincode::serialize(&data).unwrap();
            let data = bincode::deserialize(&data_bytes).unwrap();
            prover_data.push(data);
            commitments.push(commit);
        }

        let mut evaluation_claims = Rounds::new();
        for round in round_mles.iter() {
            let mut evals = Evaluations::default();
            for mle in round.iter() {
                let eval = mle.eval_at(&eval_point).await;
                evals.push(eval);
            }
            evaluation_claims.push(evals);
        }

        let proof = jagged_prover
            .prove_trusted_evaluations(
                eval_point.clone(),
                evaluation_claims.clone(),
                prover_data,
                &mut challenger,
            )
            .await
            .ok()
            .unwrap();

        (proof, commitments, evaluation_claims)
    }

    #[tokio::test]
    async fn test_jagged_verifier() {
        setup_logger();

        let row_counts_rounds = vec![
            vec![
                1 << 13,
                1 << 8,
                1 << 11,
                1 << 7,
                1 << 16,
                1 << 14,
                1 << 20,
                1 << 7,
                1 << 9,
                1 << 11,
                1 << 8,
                1 << 7,
                1 << 14,
                1 << 10,
                1 << 14,
                1 << 8,
            ],
            vec![1 << 8],
        ];
        let column_counts_rounds = vec![
            vec![47, 41, 41, 58, 52, 109, 428, 50, 53, 93, 100, 83, 31, 68, 134, 80],
            vec![512],
        ];

        let log_blowup = 1;
        let log_stacking_height = 21;
        let max_log_row_count = 20;

        let row_counts = row_counts_rounds.into_iter().collect::<Rounds<Vec<usize>>>();
        let column_counts = column_counts_rounds.into_iter().collect::<Rounds<Vec<usize>>>();

        assert!(row_counts.len() == column_counts.len());

        let mut rng = thread_rng();

        let round_mles = row_counts
            .iter()
            .zip(column_counts.iter())
            .map(|(row_counts, col_counts)| {
                row_counts
                    .iter()
                    .zip(col_counts.iter())
                    .map(|(num_rows, num_cols)| {
                        if *num_rows == 0 {
                            PaddedMle::zeros(*num_cols, max_log_row_count)
                        } else {
                            let mle = Mle::<F>::rand(&mut rng, *num_cols, num_rows.ilog(2));
                            PaddedMle::padded_with_zeros(Arc::new(mle), max_log_row_count)
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Rounds<_>>();

        let jagged_verifier = JaggedPcsVerifier::<GC, SC>::new(
            log_blowup,
            log_stacking_height,
            max_log_row_count as usize,
        );

        let eval_point = (0..max_log_row_count).map(|_| rng.gen::<EF>()).collect::<Point<_>>();

        // Generate the jagged proof.
        let (proof, mut commitments, evaluation_claims) =
            generate_jagged_proof(&jagged_verifier, round_mles, eval_point.clone()).await;

        let mut challenger = jagged_verifier.challenger();
        let machine_verifier = MachineJaggedPcsVerifier::new(
            &jagged_verifier,
            vec![column_counts[0].clone(), column_counts[1].clone()],
        );

        for commitment in commitments.iter() {
            // Ensure that the commitments are in the correct field.
            challenger.observe(*commitment);
        }

        machine_verifier
            .verify_trusted_evaluations(
                &commitments,
                eval_point.clone(),
                &evaluation_claims,
                &proof,
                &mut challenger,
            )
            .unwrap();

        // Define the verification circuit.
        let mut builder = AsmBuilder::default();
        builder.cycle_tracker_v2_enter("jagged - read input");
        let mut challenger_variable = DuplexChallengerVariable::new(&mut builder);
        let commitments_var = commitments.read(&mut builder);
        let eval_point_var = eval_point.read(&mut builder);
        let evaluation_claims_var = evaluation_claims.read(&mut builder);
        let proof_var = proof.read(&mut builder);
        builder.cycle_tracker_v2_exit();
        builder.cycle_tracker_v2_enter("jagged - observe commitments");
        for commitment_var in commitments_var.iter() {
            challenger_variable.observe_slice(&mut builder, *commitment_var);
        }
        builder.cycle_tracker_v2_exit();
        let verifier = BasefoldVerifier::<_, SP1BasefoldConfig>::new(log_blowup);
        let recursive_verifier = RecursiveBasefoldVerifier::<RecursiveBasefoldConfigImpl<C, SC>> {
            fri_config: verifier.fri_config,
            tcs: RecursiveMerkleTreeTcs::<C, SC>(PhantomData),
        };
        let recursive_verifier =
            RecursiveStackedPcsVerifier::new(recursive_verifier, log_stacking_height);

        let recursive_jagged_verifier = RecursiveJaggedPcsVerifier::<
            SC,
            C,
            RecursiveJaggedConfigImpl<
                C,
                SC,
                RecursiveBasefoldVerifier<RecursiveBasefoldConfigImpl<C, SC>>,
            >,
        > {
            stacked_pcs_verifier: recursive_verifier,
            max_log_row_count: max_log_row_count as usize,
            jagged_evaluator: RecursiveJaggedEvalSumcheckConfig::<SP1CoreJaggedConfig>(PhantomData),
        };

        let recursive_jagged_verifier = RecursiveMachineJaggedPcsVerifier::new(
            &recursive_jagged_verifier,
            vec![column_counts[0].clone(), column_counts[1].clone()],
        );

        builder.cycle_tracker_v2_enter("jagged-verifier");
        recursive_jagged_verifier.verify_trusted_evaluations(
            &mut builder,
            &commitments_var,
            eval_point_var,
            &evaluation_claims_var,
            &proof_var,
            &mut challenger_variable,
        );
        builder.cycle_tracker_v2_exit();

        let block = builder.into_root_block();
        let mut compiler = AsmCompiler::default();

        // Compile the verification circuit.
        let program = compiler.compile_inner(block).validate().unwrap();

        // Run the verification circuit with the proof artifacts.
        let mut witness_stream = Vec::new();
        Witnessable::<AsmConfig>::write(&commitments, &mut witness_stream);
        Witnessable::<AsmConfig>::write(&eval_point, &mut witness_stream);
        Witnessable::<AsmConfig>::write(&evaluation_claims, &mut witness_stream);
        Witnessable::<AsmConfig>::write(&proof, &mut witness_stream);
        let mut runtime =
            Runtime::<F, EF, SP1DiffusionMatrix>::new(Arc::new(program.clone()), inner_perm());
        runtime.witness_stream = witness_stream.into();
        runtime.run().unwrap();

        // Run the verification circuit with the proof artifacts with an expected failure.
        let mut witness_stream = Vec::new();
        commitments.rounds[0][0] += F::one();
        Witnessable::<AsmConfig>::write(&commitments, &mut witness_stream);
        Witnessable::<AsmConfig>::write(&eval_point, &mut witness_stream);
        Witnessable::<AsmConfig>::write(&evaluation_claims, &mut witness_stream);
        Witnessable::<AsmConfig>::write(&proof, &mut witness_stream);
        let mut runtime =
            Runtime::<F, EF, SP1DiffusionMatrix>::new(Arc::new(program), inner_perm());
        runtime.witness_stream = witness_stream.into();
        runtime.run().expect_err("invalid proof should not be verified");
    }
}
