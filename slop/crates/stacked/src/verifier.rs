use serde::{Deserialize, Serialize};
use slop_challenger::IopCtx;
use slop_commit::Rounds;
use slop_multilinear::{Evaluations, Mle, MultilinearPcsVerifier, Point};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct StackedPcsVerifier<GC, P> {
    pub pcs_verifier: P,
    pub log_stacking_height: u32,
    _marker: std::marker::PhantomData<GC>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum StackedVerifierError<PcsError> {
    #[error("PCS error: {0}")]
    PcsError(PcsError),
    #[error("Batch evaluations do not match the claimed evaluations")]
    StackingError,
    #[error("Proof has incorrect shape")]
    IncorrectShape,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackedPcsProof<PcsProof, EF> {
    pub pcs_proof: PcsProof,
    pub batch_evaluations: Rounds<Evaluations<EF>>,
}

impl<GC: IopCtx, P: MultilinearPcsVerifier<GC>> StackedPcsVerifier<GC, P> {
    pub fn challenger(&self) -> GC::Challenger {
        self.pcs_verifier.default_challenger()
    }

    #[inline]
    pub const fn new(pcs_verifier: P, log_stacking_height: u32) -> Self {
        Self { pcs_verifier, log_stacking_height, _marker: std::marker::PhantomData }
    }

    pub fn verify_trusted_evaluation(
        &self,
        commitments: &[GC::Digest],
        point: &Point<GC::EF>,
        proof: &StackedPcsProof<P::Proof, GC::EF>,
        evaluation_claim: GC::EF,
        challenger: &mut GC::Challenger,
    ) -> Result<(), StackedVerifierError<P::VerifierError>> {
        if point.dimension() < self.log_stacking_height as usize {
            return Err(StackedVerifierError::IncorrectShape);
        }

        // Split the point into the interleaved and batched parts.
        let (batch_point, stack_point) =
            point.split_at(point.dimension() - self.log_stacking_height as usize);

        // Interpolate the batch evaluations as a multilinear polynomial.
        let batch_evaluations =
            proof.batch_evaluations.iter().flatten().flatten().cloned().collect::<Mle<_>>();
        // Verify that the climed evaluations matched the interpolated evaluations.
        let expected_evaluation = batch_evaluations.blocking_eval_at(&batch_point)[0];
        if evaluation_claim != expected_evaluation {
            return Err(StackedVerifierError::StackingError);
        }

        // Verify the PCS proof with respect to the claimed evaluations.
        self.pcs_verifier
            .verify_untrusted_evaluations(
                commitments,
                stack_point,
                &proof.batch_evaluations,
                &proof.pcs_proof,
                challenger,
            )
            .map_err(StackedVerifierError::PcsError)
    }
}
