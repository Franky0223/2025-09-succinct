use crate::{
    utils::{is_recursion_public_values_valid, is_root_public_values_valid},
    HashableKey, OuterSC,
};
use anyhow::Result;
use num_bigint::BigUint;
use slop_algebra::{AbstractField, PrimeField, PrimeField64};
use sp1_core_executor::{subproof::SubproofVerifier, SP1RecursionProof};
use sp1_core_machine::riscv::MAX_LOG_NUMBER_OF_SHARDS;
use sp1_hypercube::{
    air::{PublicValues, POSEIDON_NUM_WORDS, PV_DIGEST_NUM_WORDS},
    MachineVerifierConfigError, MachineVerifierError, SP1CoreJaggedConfig, SP1OuterConfig,
};
use sp1_primitives::{
    io::{blake3_hash, SP1PublicValues},
    SP1Field, SP1GlobalContext, SP1OuterGlobalContext,
};
use sp1_recursion_circuit::machine::RootPublicValues;
use sp1_recursion_executor::RecursionPublicValues;
use sp1_recursion_gnark_ffi::{
    Groth16Bn254Proof, Groth16Bn254Prover, PlonkBn254Proof, PlonkBn254Prover,
};
use std::{borrow::Borrow, path::Path, str::FromStr};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PlonkVerificationError {
    #[error(
        "the verifying key does not match the inner plonk bn254 proof's committed verifying key"
    )]
    InvalidVerificationKey,
    #[error(
        "the public values in the sp1 proof do not match the public values in the inner plonk
bn254 proof"
    )]
    InvalidPublicValues,
}

#[derive(Error, Debug)]
pub enum Groth16VerificationError {
    #[error(
        "the verifying key does not match the inner groth16 bn254 proof's committed verifying
key"
    )]
    InvalidVerificationKey,
    #[error(
        "the public values in the sp1 proof do not match the public values in the inner groth16
bn254 proof"
    )]
    InvalidPublicValues,
}

impl<C: SP1ProverComponents> SP1Prover<C> {
    /// Verify a core proof by verifying the shards, verifying lookup bus, verifying that the
    /// shards are contiguous and complete. Some of the public values verification is inside the
    /// `eval_public_values` function, which is a part of the core shard proof.
    pub fn verify(
        &self,
        proof: &SP1CoreProofData,
        vk: &SP1VerifyingKey,
    ) -> Result<(), MachineVerifierConfigError<SP1GlobalContext, CoreSC>> {
        let SP1VerifyingKey { vk } = vk;

        if proof.0.is_empty() {
            return Err(MachineVerifierError::EmptyProof);
        }

        // Assert that the first shard is an execution shard and has `is_first_shard == 1`.
        // Assert that the non-first shard has `is_first_shard == 0`.
        for (i, shard_proof) in proof.0.iter().enumerate() {
            let public_values: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
                shard_proof.public_values.as_slice().borrow();
            if i == 0 && public_values.is_execution_shard != SP1Field::one() {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "first shard does not contain RISCV cycles",
                ));
            }
            if i == 0 && public_values.is_first_shard != SP1Field::one() {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "first shard should have is_first_shard == 1",
                ));
            }
            if i != 0 && public_values.is_first_shard != SP1Field::zero() {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "non-first shard should have is_first_shard == 0",
                ));
            }
        }

        // Execution shard and timestamp constraints.
        //
        // Initialization:
        // - The `is_execution_shard` and `initial_timestamp` of the first shard must be one.
        //
        // Transition:
        // - A shard's `last_timestamp` must equal the next shard's `initial_timestamp`.
        //
        // Internal Constraints:
        // - Inside the shard proof, it's constrained that `is_execution_shard == 0` if the
        // `initial_timestamp` is equal to `last_timestamp`.
        // - Inside the shard proof, it's constrained that `is_execution_shard == 1` if the
        // `initial_timestamp` is different to `last_timestamp`.
        // - Inside the shard proof, the timestamp's limbs are range checked.
        // - We include some of these checks inside the verify function for additional verification.
        let mut prev_timestamp =
            [SP1Field::zero(), SP1Field::zero(), SP1Field::zero(), SP1Field::one()];
        for shard_proof in proof.0.iter() {
            let public_values: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
                shard_proof.public_values.as_slice().borrow();
            if public_values.initial_timestamp != prev_timestamp {
                return Err(MachineVerifierError::InvalidPublicValues("invalid initial timestamp"));
            }
            // These checks below are already done in the shard proof, but done additionally.
            if public_values.is_execution_shard != SP1Field::zero()
                && public_values.initial_timestamp == public_values.last_timestamp
            {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "timestamp should change on execution shard",
                ));
            }
            if public_values.is_execution_shard != SP1Field::one()
                && public_values.initial_timestamp != public_values.last_timestamp
            {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "timestamp should not change on non-execution shard",
                ));
            }
            prev_timestamp = public_values.last_timestamp;
        }

        // Program counter constraints.
        //
        // Initialization:
        // - `pc_start` should start as `vk.pc_start`.
        //
        // Transition:
        // - `next_pc` of a shard should equal to `pc_start` of the next shard.
        //
        // Finalization:
        // - `next_pc` should equal `HALT_PC`.
        //
        // Internal Constraints:
        // - Inside the shard proof, it's constrained that `pc_start` and `next_pc` are composed of
        // valid u16 limbs, and that in a non-execution shard, `pc_start` and `next_pc` are equal.
        // - We include some of these checks inside the verify function for additional verification.
        let mut prev_next_pc = vk.pc_start;
        let halt_pc = [
            SP1Field::from_canonical_u64(sp1_core_executor::HALT_PC),
            SP1Field::zero(),
            SP1Field::zero(),
        ];
        for (i, shard_proof) in proof.0.iter().enumerate() {
            let public_values: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
                shard_proof.public_values.as_slice().borrow();
            if public_values.pc_start != prev_next_pc {
                if i == 0 {
                    return Err(MachineVerifierError::InvalidPublicValues(
                        "pc_start != vk.pc_start: program counter should start at vk.pc_start",
                    ));
                } else {
                    return Err(MachineVerifierError::InvalidPublicValues(
                        "pc_start != prev_next_pc: pc_start should equal prev_next_pc for all shards",
                    ));
                }
            }
            // These checks below are already done in the shard proof, but done additionally.
            if public_values.is_execution_shard != SP1Field::one()
                && public_values.pc_start != public_values.next_pc
            {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "pc_start != next_pc: pc_start should equal next_pc for non-execution shards",
                ));
            }
            prev_next_pc = public_values.next_pc;
        }
        if prev_next_pc != halt_pc {
            return Err(MachineVerifierError::InvalidPublicValues(
                "next_pc != HALT_PC: execution should have halted",
            ));
        }

        // Exit code constraints.
        //
        // Initialization
        // - The `prev_exit_code` of the first shard must be zero.
        //
        // Transition
        // - The `exit_code` of the shard must be the `prev_exit_code` of the next shard.
        //
        // Internal Constraints:
        // - Inside the shard proof, it is constrained that `prev_exit_code` and `exit_code` must
        // equal to each other if the shard is not an execution shard.
        // - Inside the shard proof, it is constrained that if `prev_exit_code` is non-zero, then
        // `exit_code` must be equal to the `prev_exit_code`.
        // - We include these checks inside the verify function for additional verification.
        let mut prev_exit_code = SP1Field::zero();
        for shard_proof in proof.0.iter() {
            let public_values: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
                shard_proof.public_values.as_slice().borrow();
            if public_values.prev_exit_code != prev_exit_code {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "public_values.prev_exit_code != prev_exit_code: prev_exit_code does not match previous shard's exit_code",
                ));
            }
            // These checks below are already done in the shard proof, but done additionally.
            if public_values.is_execution_shard != SP1Field::one()
                && public_values.prev_exit_code != public_values.exit_code
            {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "prev_exit_code != exit_code: exit code should be same in non-execution shards",
                ));
            }
            if public_values.prev_exit_code != SP1Field::zero()
                && public_values.prev_exit_code != public_values.exit_code
            {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "prev_exit_code != exit_code: exit code should change at most once",
                ));
            }
            prev_exit_code = public_values.exit_code;
        }

        // Proof nonce constraints.
        //
        // Proof nonce value should be same across all shards
        let public_values_first_shard: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
            proof.0.first().unwrap().public_values.as_slice().borrow();
        let proof_nonce_first_shard = public_values_first_shard.proof_nonce;
        for shard_proof in proof.0[1..].iter() {
            let public_values: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
                shard_proof.public_values.as_slice().borrow();
            if public_values.proof_nonce != proof_nonce_first_shard {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "proof_nonce != proof_nonce_first_shard",
                ));
            }
        }

        // Memory initialization & finalization constraints.
        //
        // Initialization:
        // - `previous_init_addr` should be zero.
        // - `previous_finalize_addr` should be zero.
        // - `previous_init_page_idx` should be zero.
        // - `previous_finalize_page_idx` should be zero.
        //
        // Transition:
        // - The `previous_init_addr` should equal `last_init_addr` of the previous shard.
        // - The `previous_finalize_addr` should equal `last_finalize_addr` of the previous shard.
        // - The `previous_init_page_idx` should be `last_init_page_idx` of the previous shard.
        // - The `previous_finalize_page_idx` is `last_finalize_page_idx` of the previous shard.
        //
        // Finalization:
        // - The final `last_init_addr` should be non-zero.
        // - The final `last_finalize_addr` should be non-zero.
        //
        // Internal Constraints:
        // - Inside the shard proof, it is constrained that the addresses are of valid u16 limbs.
        let mut last_init_addr_prev = [SP1Field::zero(); 3];
        let mut last_finalize_addr_prev = [SP1Field::zero(); 3];
        let mut last_init_page_idx_prev = [SP1Field::zero(); 3];
        let mut last_finalize_page_idx_prev = [SP1Field::zero(); 3];

        for shard_proof in proof.0.iter() {
            let public_values: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
                shard_proof.public_values.as_slice().borrow();

            if public_values.previous_init_addr != last_init_addr_prev {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "previous_init_addr != last_init_addr_prev",
                ));
            } else if public_values.previous_finalize_addr != last_finalize_addr_prev {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "previous_finalize_addr != last_finalize_addr_prev",
                ));
            } else if public_values.previous_init_page_idx != last_init_page_idx_prev {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "previous_init_page_idx != last_init_page_idx_prev",
                ));
            } else if public_values.previous_finalize_page_idx != last_finalize_page_idx_prev {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "previous_finalize_page_idx != last_finalize_page_idx_prev",
                ));
            } else if public_values.is_untrusted_programs_enabled != vk.enable_untrusted_programs {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "public_values.is_untrusted_programs_enabled != vk.enable_untrusted_programs",
                ));
            }

            last_init_addr_prev = public_values.last_init_addr;
            last_finalize_addr_prev = public_values.last_finalize_addr;
            last_init_page_idx_prev = public_values.last_init_page_idx;
            last_finalize_page_idx_prev = public_values.last_finalize_page_idx;
        }
        if last_init_addr_prev == [SP1Field::zero(); 3] {
            return Err(MachineVerifierError::InvalidPublicValues(
                "the zero address was never initialized",
            ));
        }
        if last_finalize_addr_prev == [SP1Field::zero(); 3] {
            return Err(MachineVerifierError::InvalidPublicValues(
                "the zero address was never finalized",
            ));
        }

        // Digest constraints.
        //
        // Initialization:
        // - `prev_committed_value_digest` should be zero.
        // - `prev_deferred_proofs_digest` should be zero.
        // - `prev_commit_syscall` should be zero.
        // - `prev_commit_deferred_syscall` should be zero.
        //
        // Transition:
        // - `committed_value_digest` must equal the next shard's `prev_committed_value_digest`.
        // - `deferred_proofs_digest` must equal the next shard's `prev_deferred_proofs_digest`.
        // - `commit_syscall` must equal the next shard's `prev_commit_syscall`.
        // - `commit_deferred_syscall` must equal the next shard's `prev_commit_deferred_syscall`.
        //
        // Finalization:
        // - The last `commit_syscall` should equal one.
        // - The last `commit_deferred_syscall` should equal one.
        //
        // Internal Constraints for `committed_value_digest` and `commit_syscall`:
        // - The `prev_committed_value_digest` are of valid bytes.
        // - The `committed_value_digest` are of valid bytes.
        // - If the `COMMIT` syscall was called in the shard, then `commit_syscall == 1`.
        // - `prev_commit_syscall` and `commit_syscall` are boolean.
        // - If `prev_commit_syscall == 1`, then `commit_syscall == 1`.
        // - In a non-execution shard, `prev_commit_syscall == commit_syscall`.
        // - If the shard isn't an execution shard, or has `prev_commit_syscall == 1`, or if
        // `prev_committed_value_digest` has a non-zero byte inside it, then
        // `prev_committed_value_digest == committed_value_digest`.
        //
        // Internal Constraints for `deferred_proofs_digest` and `commit_deferred_syscall`:
        // - If `COMMIT_DEFERRED_PROOFS` syscall was called, `commit_deferred_syscall == 1`.
        // - `prev_commit_deferred_syscall` and `commit_deferred_syscall` are boolean.
        // - If `prev_commit_deferred_syscall == 1`, then `commit_deferred_syscall == 1`.
        // - In a non-execution shard, `prev_commit_deferred_syscall == commit_deferred_syscall`.
        // - If the shard isn't an execution shard, or has `prev_commit_deferred_syscall == 1`, or
        // if `prev_deferred_proofs_digest` has a non-zero limb inside it, then
        // `prev_deferred_proofs_digest == deferred_proofs_digest`.
        let zero_committed_value_digest = [[SP1Field::zero(); 4]; PV_DIGEST_NUM_WORDS];
        let zero_deferred_proofs_digest = [SP1Field::zero(); POSEIDON_NUM_WORDS];
        let mut commit_syscall_prev = SP1Field::zero();
        let mut commit_deferred_syscall_prev = SP1Field::zero();
        let mut committed_value_digest_prev = zero_committed_value_digest;
        let mut deferred_proofs_digest_prev = zero_deferred_proofs_digest;
        for shard_proof in proof.0.iter() {
            let public_values: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
                shard_proof.public_values.as_slice().borrow();
            if public_values.prev_committed_value_digest != committed_value_digest_prev {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "prev_committed_value_digest doesn't equal the previous shard's committed_value_digest",
                ));
            }
            if public_values.prev_deferred_proofs_digest != deferred_proofs_digest_prev {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "prev_deferred_proofs_digest doesn't equal the previous shard's deferred_proofs_digest",
                ));
            }
            if public_values.prev_commit_syscall != commit_syscall_prev {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "prev_commit_syscall doesn't equal the previous shard's commit_syscall",
                ));
            }
            if public_values.prev_commit_deferred_syscall != commit_deferred_syscall_prev {
                return Err(MachineVerifierError::InvalidPublicValues(
                    "prev_commit_deferred_syscall doesn't equal the previous shard's commit_deferred_syscall",
                ));
            }
            committed_value_digest_prev = public_values.committed_value_digest;
            deferred_proofs_digest_prev = public_values.deferred_proofs_digest;
            commit_syscall_prev = public_values.commit_syscall;
            commit_deferred_syscall_prev = public_values.commit_deferred_syscall;
        }
        if commit_syscall_prev != SP1Field::one() {
            return Err(MachineVerifierError::InvalidPublicValues(
                "COMMIT syscall was never called",
            ));
        }
        if commit_deferred_syscall_prev != SP1Field::one() {
            return Err(MachineVerifierError::InvalidPublicValues(
                "COMMIT_DEFERRED_PROOFS syscall was never called",
            ));
        }

        // Verify that the number of shards is not too large.
        if proof.0.len() >= 1 << MAX_LOG_NUMBER_OF_SHARDS {
            return Err(MachineVerifierError::TooManyShards);
        }

        // Verify the global cumulative sum is correct.
        let initial_global_cumulative_sum = vk.initial_global_cumulative_sum;
        let mut cumulative_sum = initial_global_cumulative_sum;
        for shard_proof in proof.0.iter() {
            let public_values: &PublicValues<[_; 4], [_; 3], [_; 4], _> =
                shard_proof.public_values.as_slice().borrow();

            cumulative_sum = cumulative_sum + public_values.global_cumulative_sum;
        }
        if !cumulative_sum.is_zero() {
            return Err(MachineVerifierError::InvalidPublicValues(
                "global cumulative sum is not zero",
            ));
        }

        // Verify the shard proofs.
        for (i, shard_proof) in proof.0.iter().enumerate() {
            let span = tracing::debug_span!("Verify shard proof", i).entered();
            let mut challenger = self.core_prover.verifier().challenger();
            vk.observe_into(&mut challenger);
            self.core_prover
                .verifier()
                .verify_shard(vk, shard_proof, &mut challenger)
                .map_err(MachineVerifierError::InvalidShardProof)?;
            span.exit();
        }

        Ok(())
    }

    /// Verify a compressed proof.
    pub fn verify_compressed(
        &self,
        proof: &SP1RecursionProof<SP1GlobalContext, SP1CoreJaggedConfig>,
        vk: &SP1VerifyingKey,
    ) -> Result<(), MachineVerifierConfigError<SP1GlobalContext, CoreSC>> {
        let SP1RecursionProof { vk: compress_vk, proof } = proof;
        let mut challenger = self.recursion_prover.verifier().challenger();
        compress_vk.observe_into(&mut challenger);

        // Verify the shard proof.
        self.recursion_prover
            .verifier()
            .verify_shard(compress_vk, proof, &mut challenger)
            .map_err(MachineVerifierError::InvalidShardProof)?;

        // Validate the public values.
        let public_values: &RecursionPublicValues<_> = proof.public_values.as_slice().borrow();

        // The `digest` is the correct hash of the recursion public values.
        if !is_recursion_public_values_valid(public_values) {
            return Err(MachineVerifierError::InvalidPublicValues(
                "recursion public values are invalid",
            ));
        }

        // The `vk_root` is the expected `vk_root`.
        if public_values.vk_root != self.recursion_prover.recursion_vk_root {
            return Err(MachineVerifierError::InvalidPublicValues("vk_root mismatch"));
        }

        // If `vk_verification` is on, check the `vk` is within the expected list of `vk`'s.
        // This `vk_verification` must be only turned off for testing purposes.
        if self.recursion_prover.vk_verification()
            && !self.recursion_prover.recursion_vk_map.contains_key(&compress_vk.hash_koalabear())
        {
            return Err(MachineVerifierError::InvalidVerificationKey);
        }

        // `is_complete` should be 1. This ensures that the proof is fully reduced.
        if public_values.is_complete != SP1Field::one() {
            return Err(MachineVerifierError::InvalidPublicValues("is_complete is not 1"));
        }

        // Verify that the proof is for the sp1 vkey we are expecting.
        let vkey_hash = vk.hash_koalabear();
        if public_values.sp1_vk_digest != vkey_hash {
            return Err(MachineVerifierError::InvalidPublicValues("sp1 vk hash mismatch"));
        }

        Ok(())
    }

    /// Verify a shrink proof.
    pub fn verify_shrink(
        &self,
        proof: &SP1RecursionProof<SP1GlobalContext, SP1CoreJaggedConfig>,
        vk: &SP1VerifyingKey,
    ) -> Result<(), MachineVerifierConfigError<SP1GlobalContext, CoreSC>> {
        let SP1RecursionProof { vk: _, proof } = proof;
        let shrink_vk = self.recursion_prover.get_shrink_keys().1;
        let mut challenger = self.recursion_prover.shrink_verifier().challenger();
        shrink_vk.observe_into(&mut challenger);

        // Verify the shard proof.
        self.recursion_prover
            .shrink_verifier()
            .verify_shard(&shrink_vk, proof, &mut challenger)
            .map_err(MachineVerifierError::InvalidShardProof)?;

        // Validate public values.
        let public_values: &RecursionPublicValues<_> = proof.public_values.as_slice().borrow();

        // The `digest` is the correct hash of the recursion public values.
        if !is_recursion_public_values_valid(public_values) {
            return Err(MachineVerifierError::InvalidPublicValues(
                "recursion public values are invalid",
            ));
        }

        // The `vk_root` is the expected `vk_root`.
        if public_values.vk_root != self.recursion_prover.recursion_vk_root {
            return Err(MachineVerifierError::InvalidPublicValues("vk_root mismatch"));
        }

        // If `vk_verification` is on, check the `vk` is within the expected list of `vk`'s.
        // This `vk_verification` must be only turned off for testing purposes.
        if self.recursion_prover.vk_verification()
            && !self.recursion_prover.recursion_vk_map.contains_key(&shrink_vk.hash_koalabear())
        {
            return Err(MachineVerifierError::InvalidVerificationKey);
        }

        // `is_complete` should be 1. This ensures that the proof is fully reduced.
        if public_values.is_complete != SP1Field::one() {
            return Err(MachineVerifierError::InvalidPublicValues("is_complete is not 1"));
        }

        // Verify that the proof is for the sp1 vkey we are expecting.
        let vkey_hash = vk.hash_koalabear();
        if public_values.sp1_vk_digest != vkey_hash {
            return Err(MachineVerifierError::InvalidPublicValues("sp1 vk hash mismatch"));
        }

        Ok(())
    }

    /// Verify a wrap bn254 proof.
    pub fn verify_wrap_bn254(
        &self,
        proof: &SP1RecursionProof<SP1OuterGlobalContext, SP1OuterConfig>,
        vk: &SP1VerifyingKey,
    ) -> Result<(), MachineVerifierConfigError<SP1OuterGlobalContext, OuterSC>> {
        let SP1RecursionProof { vk: _, proof } = proof;
        let wrap_vk = self.recursion_prover.get_wrap_keys().1;
        let mut challenger = self.recursion_prover.wrap_verifier().challenger();
        wrap_vk.observe_into(&mut challenger);

        // Verify the shard proof.
        self.recursion_prover
            .wrap_verifier()
            .verify_shard(&wrap_vk, proof, &mut challenger)
            .map_err(MachineVerifierError::InvalidShardProof)?;

        // Validate public values.
        let public_values: &RootPublicValues<_> = proof.public_values.as_slice().borrow();
        if !is_root_public_values_valid(public_values) {
            return Err(MachineVerifierError::InvalidPublicValues(
                "root public values are invalid",
            ));
        }

        // Verify that the proof is for the sp1 vkey we are expecting.
        let vkey_hash = vk.hash_koalabear();
        if *public_values.sp1_vk_digest() != vkey_hash {
            return Err(MachineVerifierError::InvalidPublicValues("sp1 vk hash mismatch"));
        }

        Ok(())
    }

    /// Verifies a PLONK proof using the circuit artifacts in the build directory.
    pub fn verify_plonk_bn254(
        &self,
        proof: &PlonkBn254Proof,
        vk: &SP1VerifyingKey,
        public_values: &SP1PublicValues,
        build_dir: &Path,
    ) -> Result<()> {
        let prover = PlonkBn254Prover::new();

        let vkey_hash = BigUint::from_str(&proof.public_inputs[0])?;
        let committed_values_digest = BigUint::from_str(&proof.public_inputs[1])?;
        let exit_code = BigUint::from_str(&proof.public_inputs[2])?;
        let vk_root = BigUint::from_str(&proof.public_inputs[3])?;
        let proof_nonce = BigUint::from_str(&proof.public_inputs[4])?;

        // Verify the proof with the corresponding public inputs.
        prover.verify(
            proof,
            &vkey_hash,
            &committed_values_digest,
            &exit_code,
            &vk_root,
            &proof_nonce,
            build_dir,
        )?;

        verify_plonk_bn254_public_inputs(vk, public_values, &proof.public_inputs)?;

        Ok(())
    }

    /// Verifies a Groth16 proof using the circuit artifacts in the build directory.
    pub fn verify_groth16_bn254(
        &self,
        proof: &Groth16Bn254Proof,
        vk: &SP1VerifyingKey,
        public_values: &SP1PublicValues,
        build_dir: &Path,
    ) -> Result<()> {
        let prover = Groth16Bn254Prover::new();

        let vkey_hash = BigUint::from_str(&proof.public_inputs[0])?;
        let committed_values_digest = BigUint::from_str(&proof.public_inputs[1])?;
        let exit_code = BigUint::from_str(&proof.public_inputs[2])?;
        let vk_root = BigUint::from_str(&proof.public_inputs[3])?;
        let proof_nonce = BigUint::from_str(&proof.public_inputs[4])?;

        // Verify the proof with the corresponding public inputs.
        prover.verify(
            proof,
            &vkey_hash,
            &committed_values_digest,
            &exit_code,
            &vk_root,
            &proof_nonce,
            build_dir,
        )?;

        verify_groth16_bn254_public_inputs(vk, public_values, &proof.public_inputs)?;

        Ok(())
    }
}

/// Verify the vk_hash and public_values_hash in the public inputs of the PlonkBn254Proof match
/// the expected values.
pub fn verify_plonk_bn254_public_inputs(
    vk: &SP1VerifyingKey,
    public_values: &SP1PublicValues,
    plonk_bn254_public_inputs: &[String],
) -> Result<()> {
    let expected_vk_hash = BigUint::from_str(&plonk_bn254_public_inputs[0])?;
    let expected_public_values_hash = BigUint::from_str(&plonk_bn254_public_inputs[1])?;

    let vk_hash = vk.hash_bn254().as_canonical_biguint();
    if vk_hash != expected_vk_hash {
        return Err(PlonkVerificationError::InvalidVerificationKey.into());
    }

    let public_values_hash = public_values.hash_bn254();
    if public_values_hash != expected_public_values_hash {
        return Err(PlonkVerificationError::InvalidPublicValues.into());
    }

    Ok(())
}

/// Verify the vk_hash and public_values_hash in the public inputs of the Groth16Bn254Proof
/// match the expected values.
pub fn verify_groth16_bn254_public_inputs(
    vk: &SP1VerifyingKey,
    public_values: &SP1PublicValues,
    groth16_bn254_public_inputs: &[String],
) -> Result<()> {
    let expected_vk_hash = BigUint::from_str(&groth16_bn254_public_inputs[0])?;
    let expected_public_values_hash = BigUint::from_str(&groth16_bn254_public_inputs[1])?;

    let vk_hash = vk.hash_bn254().as_canonical_biguint();
    if vk_hash != expected_vk_hash {
        return Err(Groth16VerificationError::InvalidVerificationKey.into());
    }

    let public_values_hash = public_values.hash_bn254();
    if public_values_hash != expected_public_values_hash {
        return Err(Groth16VerificationError::InvalidPublicValues.into());
    }
    verify_public_values(public_values, expected_public_values_hash)?;

    Ok(())
}

/// In SP1, a proof's public values can either be hashed with SHA2 or Blake3. In SP1 V4, there is no
/// metadata attached to the proof about which hasher function was used for public values hashing.
/// Instead, when verifying the proof, the public values are hashed with SHA2 and Blake3, and
/// if either matches the `expected_public_values_hash`, the verification is successful.
///
/// The security for this verification in SP1 V4 derives from the fact that both SHA2 and Blake3 are
/// designed to be collision resistant. It is computationally infeasible to find an input i1 for
/// SHA256 and an input i2 for Blake3 that the same hash value. Doing so would require breaking both
/// algorithms simultaneously.
fn verify_public_values(
    public_values: &SP1PublicValues,
    expected_public_values_hash: BigUint,
) -> Result<()> {
    // First, check if the public values are hashed with SHA256. If that fails, attempt hashing with
    // Blake3. If neither match, return an error.
    let sha256_public_values_hash = public_values.hash_bn254();
    if sha256_public_values_hash != expected_public_values_hash {
        let blake3_public_values_hash = public_values.hash_bn254_with_fn(blake3_hash);
        if blake3_public_values_hash != expected_public_values_hash {
            return Err(Groth16VerificationError::InvalidPublicValues.into());
        }
    }

    Ok(())
}

use crate::{
    components::SP1ProverComponents, CoreSC, InnerSC, SP1CoreProofData, SP1Prover, SP1VerifyingKey,
};

impl<C: SP1ProverComponents> SubproofVerifier for SP1Prover<C> {
    fn verify_deferred_proof(
        &self,
        proof: &sp1_core_machine::recursion::SP1RecursionProof<SP1GlobalContext, InnerSC>,
        vk: &sp1_hypercube::MachineVerifyingKey<SP1GlobalContext, CoreSC>,
        vk_hash: [u64; 4],
        committed_value_digest: [u64; 4],
    ) -> Result<(), MachineVerifierConfigError<SP1GlobalContext, CoreSC>> {
        // Check that the vk hash matches the vk hash from the input.
        if vk.hash_u64() != vk_hash {
            return Err(MachineVerifierError::InvalidPublicValues(
                "vk hash from syscall does not match vkey from input",
            ));
        }
        // Check that proof is valid.
        self.verify_compressed(
            &SP1RecursionProof { vk: proof.vk.clone(), proof: proof.proof.clone() },
            &SP1VerifyingKey { vk: vk.clone() },
        )?;
        // Check that the committed value digest matches the one from syscall
        let public_values: &RecursionPublicValues<_> =
            proof.proof.public_values.as_slice().borrow();
        let pv_committed_value_digest: [u64; 4] = std::array::from_fn(|i| {
            public_values.committed_value_digest[2 * i]
                .iter()
                .chain(public_values.committed_value_digest[2 * i + 1].iter())
                .enumerate()
                .fold(0u64, |acc, (j, &val)| acc | (val.as_canonical_u64() << (8 * j)))
        });

        if committed_value_digest != pv_committed_value_digest {
            return Err(MachineVerifierError::InvalidPublicValues(
                "committed_value_digest does not match",
            ));
        }

        Ok(())
    }
}
