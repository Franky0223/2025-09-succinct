use std::ops::{Deref, DerefMut};
use std::{error::Error, fmt::Debug, future::Future};

use crate::{Mle, MleEval, MleEvaluationBackend, Point};
use derive_where::derive_where;
use serde::{de::DeserializeOwned, Serialize};
use slop_algebra::{ExtensionField, Field};
use slop_alloc::ToHost;
use slop_alloc::{Backend, CpuBackend, HasBackend};
use slop_challenger::{FieldChallenger, IopCtx};
use slop_commit::{Message, Rounds};

#[derive(Debug, Clone)]
#[derive_where(PartialEq, Eq, Serialize, Deserialize; MleEval<F, A>)]
pub struct Evaluations<F, A: Backend = CpuBackend> {
    pub round_evaluations: Vec<MleEval<F, A>>,
}

/// A verifier of a multilinear commitment scheme.
///
/// A verifier for a multilinear commitment scheme (or PCS) is a protocol that enables getting
/// succinct commitments representing multiplinear polynomials and later making query checks for
/// their evaluation.
///
/// The verifier described by this trait supports compiling a multi-stage multilinear polynomial
/// IOP. In each round of the protocol, the prover is allowed to send a commitment of type
/// [MultilinearPcsVerifier::Commitment] which represents a batch of multilinear polynomials. After
/// all the rounds are complete, the verifier can check an evaluation claim for all the polynomials
/// in all rounds, evaluated at same [Point].
pub trait MultilinearPcsVerifier<GC: IopCtx>: 'static + Send + Sync + Clone {
    /// The proof of a multilinear PCS evaluation.
    type Proof: 'static + Clone + Serialize + DeserializeOwned + Send + Sync;

    /// The error type of the verifier.
    type VerifierError: Error;

    /// A default challenger for Fiat-Shamir.
    ///
    /// The challenger returned by this method is un-seeded and it's state can be determinstic.
    fn default_challenger(&self) -> GC::Challenger;

    /// Verify an evaluation proofs for multilinear polynomials sent.
    ///
    /// All inputs are assumed to "trusted" in the sense of Fiat-Shamir. Namely, it is assumed that
    /// the inputs have already been absorbed into the Fiat-Shamir randomness represented by the
    /// challenger.
    ///
    /// ### Arguments
    ///
    /// * `commitments` - The commitments to the multilinear polynomials sent by the prover. A
    ///   commitment is sent for each round of the protocol.
    /// * `point` - The evaluation point at which the multilinear polynomials are evaluated.
    /// * `evaluation_claims` - The evaluation claims for the multilinear polynomials. the slice
    ///   contains one [MleEval] for each round of the protocol.
    /// * `proof` - The proof of the evaluation claims.
    /// * `challenger` - The challenger that creates the verifier messages of the IOP.
    fn verify_trusted_evaluations(
        &self,
        commitments: &[GC::Digest],
        point: Point<GC::EF>,
        evaluation_claims: &[Evaluations<GC::EF>],
        proof: &Self::Proof,
        challenger: &mut GC::Challenger,
    ) -> Result<(), Self::VerifierError>;

    /// Verify an evaluation proof for a multilinear polynomial.
    ///
    /// This is a variant of [MultilinearPcsVerifier::verify_trusted_evaluations] that allows the
    /// evaluations to be "untrusted" in the sense of Fiat-Shamir. Namely, the verifier will first
    /// absorb the evaluation claims into the Fiat-Shamir randomness represented by the challenger.
    fn verify_untrusted_evaluations(
        &self,
        commitments: &[GC::Digest],
        point: Point<GC::EF>,
        evaluation_claims: &[Evaluations<GC::EF>],
        proof: &Self::Proof,
        challenger: &mut GC::Challenger,
    ) -> Result<(), Self::VerifierError> {
        // Observe the evaluation claims.
        for round in evaluation_claims.iter() {
            for round_evaluations in round.iter() {
                for evaluations in round_evaluations.iter() {
                    for evaluation in evaluations.iter() {
                        challenger.observe_ext_element(*evaluation);
                    }
                }
            }
        }

        self.verify_trusted_evaluations(commitments, point, evaluation_claims, proof, challenger)
    }
}

/// The prover of a multilinear commitment scheme.
pub trait MultilinearPcsProver<GC: IopCtx>: 'static + Debug + Send + Sync {
    /// The verifier associated to this prover.
    type Verifier: MultilinearPcsVerifier<GC>;

    /// The auxilary data for a prover.
    ///
    /// When committing to a batch of multilinear polynomials, it is often necessary to keep track
    /// of additional information that was produced during the commitment phase.
    type ProverData: 'static + Send + Sync + Debug + Clone;

    /// The backend used by the prover.
    ///
    /// The backend parametrizes the type of hardware assumptions this prover is using.
    type A: MleEvaluationBackend<GC::F, GC::EF>;

    /// The error type of the prover.
    type ProverError: Error;

    fn commit_multilinears(
        &self,
        mles: Message<Mle<GC::F, Self::A>>,
    ) -> impl Future<Output = Result<(GC::Digest, Self::ProverData), Self::ProverError>> + Send;

    fn prove_trusted_evaluations(
        &self,
        eval_point: Point<GC::EF>,
        mle_rounds: Rounds<Message<Mle<GC::F, Self::A>>>,
        evaluation_claims: Rounds<Evaluations<GC::EF, Self::A>>,
        prover_data: Rounds<Self::ProverData>,
        challenger: &mut GC::Challenger,
    ) -> impl Future<
        Output = Result<<Self::Verifier as MultilinearPcsVerifier<GC>>::Proof, Self::ProverError>,
    > + Send;

    fn prove_untrusted_evaluations(
        &self,
        eval_point: Point<GC::EF>,
        mle_rounds: Rounds<Message<Mle<GC::F, Self::A>>>,
        evaluation_claims: Rounds<Evaluations<GC::EF, Self::A>>,
        prover_data: Rounds<Self::ProverData>,
        challenger: &mut GC::Challenger,
    ) -> impl Future<
        Output = Result<<Self::Verifier as MultilinearPcsVerifier<GC>>::Proof, Self::ProverError>,
    > + Send {
        async {
            // Observe the evaluation claims.
            for round in evaluation_claims.iter() {
                for claim in round.iter() {
                    let host_claim = claim.to_host().await.unwrap();
                    // let host_claim = Self::A::evaluations_to_host(claim).await.unwrap();
                    for evaluations in host_claim.iter() {
                        for evaluation in evaluations.iter() {
                            challenger.observe_ext_element(*evaluation);
                        }
                    }
                }
            }

            self.prove_trusted_evaluations(
                eval_point,
                mle_rounds,
                evaluation_claims,
                prover_data,
                challenger,
            )
            .await
        }
    }
}

impl<F, A: Backend> IntoIterator for Evaluations<F, A> {
    type Item = MleEval<F, A>;
    type IntoIter = <Vec<MleEval<F, A>> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.round_evaluations.into_iter()
    }
}

impl<'a, F, A: Backend> IntoIterator for &'a Evaluations<F, A> {
    type Item = &'a MleEval<F, A>;
    type IntoIter = std::slice::Iter<'a, MleEval<F, A>>;

    fn into_iter(self) -> Self::IntoIter {
        self.round_evaluations.iter()
    }
}

impl<F, A: Backend> Evaluations<F, A> {
    #[inline]
    pub fn iter(&'_ self) -> std::slice::Iter<'_, MleEval<F, A>> {
        self.round_evaluations.iter()
    }

    #[inline]
    pub const fn new(round_evaluations: Vec<MleEval<F, A>>) -> Self {
        Self { round_evaluations }
    }
}

impl<F, A: Backend> FromIterator<MleEval<F, A>> for Evaluations<F, A> {
    fn from_iter<T: IntoIterator<Item = MleEval<F, A>>>(iter: T) -> Self {
        Self { round_evaluations: iter.into_iter().collect() }
    }
}

impl<F, A: Backend> Extend<MleEval<F, A>> for Evaluations<F, A> {
    fn extend<T: IntoIterator<Item = MleEval<F, A>>>(&mut self, iter: T) {
        self.round_evaluations.extend(iter);
    }
}

impl<F, A> HasBackend for Evaluations<F, A>
where
    A: Backend,
{
    type Backend = A;

    fn backend(&self) -> &Self::Backend {
        assert!(!self.round_evaluations.is_empty(), "Evaluations must not be empty");
        self.round_evaluations.first().unwrap().backend()
    }
}

impl<F, A: Backend> Default for Evaluations<F, A> {
    fn default() -> Self {
        Self { round_evaluations: Vec::new() }
    }
}

impl<F, A: Backend> Deref for Evaluations<F, A> {
    type Target = Vec<MleEval<F, A>>;

    fn deref(&self) -> &Self::Target {
        &self.round_evaluations
    }
}

impl<F, A: Backend> DerefMut for Evaluations<F, A> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.round_evaluations
    }
}

pub trait MultilinearPcsChallenger<F: Field>: FieldChallenger<F> {
    fn sample_point<EF: ExtensionField<F>>(&mut self, num_variables: u32) -> Point<EF> {
        (0..num_variables).map(|_| self.sample_ext_element::<EF>()).collect()
    }
}

impl<F: Field, C> MultilinearPcsChallenger<F> for C where C: FieldChallenger<F> {}
