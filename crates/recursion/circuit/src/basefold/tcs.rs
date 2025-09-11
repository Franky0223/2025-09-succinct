use crate::{basefold::merkle_tree::verify, hash::FieldHasherVariable, AsRecursive, CircuitConfig};
use itertools::Itertools;
use slop_bn254::BNGC;
use slop_merkle_tree::{MerkleTreeTcs, Poseidon2Bn254Config};
use slop_tensor::Tensor;
use sp1_hypercube::{SP1CoreJaggedConfig, SP1MerkleTreeConfig, SP1OuterConfig};
use sp1_primitives::{SP1ExtensionField, SP1Field, SP1GlobalContext};
use sp1_recursion_compiler::ir::{Builder, Felt, IrIter};
use std::marker::PhantomData;

// pub trait RecursiveTcs: Sized {
//     type Data;
//     type Commitment;
//     type Circuit: CircuitConfig<Bit = Self::Bit>;
//     type Bit;

//     fn verify_tensor_openings(
//         builder: &mut Builder<Self::Circuit>,
//         commit: &Self::Commitment,
//         indices: &[Vec<Self::Bit>],
//         opening: &RecursiveTensorCsOpening<Self>,
//     );
// }

/// An opening of a tensor commitment scheme.
pub struct RecursiveTensorCsOpening<CommitmentVariable> {
    /// The claimed values of the opening.
    pub values: Tensor<Felt<SP1Field>>,
    /// The proof of the opening.
    pub proof: Tensor<CommitmentVariable>,
}

#[derive(Debug, Copy, PartialEq, Eq)]
pub struct RecursiveMerkleTreeTcs<C, M>(pub PhantomData<(C, M)>);

impl<C, M> Clone for RecursiveMerkleTreeTcs<C, M> {
    fn clone(&self) -> Self {
        Self(PhantomData)
    }
}

impl<C: CircuitConfig> AsRecursive<C> for MerkleTreeTcs<SP1GlobalContext, SP1MerkleTreeConfig> {
    type Recursive = RecursiveMerkleTreeTcs<C, SP1CoreJaggedConfig>;
}

impl<C: CircuitConfig> AsRecursive<C>
    for MerkleTreeTcs<BNGC<SP1Field, SP1ExtensionField>, Poseidon2Bn254Config<SP1Field>>
{
    type Recursive = RecursiveMerkleTreeTcs<C, SP1OuterConfig>;
}

impl<C, M> RecursiveMerkleTreeTcs<C, M>
where
    C: CircuitConfig,
    M: FieldHasherVariable<C>,
{
    pub fn verify_tensor_openings(
        builder: &mut Builder<C>,
        commit: &M::DigestVariable,
        indices: &[Vec<C::Bit>],
        opening: &RecursiveTensorCsOpening<M::DigestVariable>,
    ) {
        let chunk_size = indices.len().div_ceil(8);
        indices
            .iter()
            .zip_eq(opening.proof.split())
            .chunks(chunk_size)
            .into_iter()
            .enumerate()
            .ir_par_map_collect::<Vec<_>, _, _>(builder, |builder, (i, chunk)| {
                for (j, (index, path)) in chunk.into_iter().enumerate() {
                    let claimed_values_slices =
                        opening.values.get(i * chunk_size + j).unwrap().as_slice().to_vec();

                    let path = path.as_slice().to_vec();
                    let digest = M::hash(builder, &claimed_values_slices);

                    verify::<C, M>(builder, path, index.to_vec(), digest, *commit);
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use rand::{thread_rng, Rng};
    use slop_commit::Message;
    use slop_merkle_tree::{ComputeTcsOpenings, MerkleTreeOpening, TensorCsProver};
    use sp1_hypercube::inner_perm;
    use sp1_recursion_compiler::circuit::AsmConfig;
    use std::sync::Arc;

    use slop_algebra::extension::BinomialExtensionField;
    use sp1_primitives::SP1DiffusionMatrix;

    use crate::witness::Witnessable;

    use super::*;
    use itertools::Itertools;
    use slop_tensor::Tensor;
    use sp1_hypercube::prover::SP1MerkleTreeProver;
    use sp1_recursion_compiler::circuit::{AsmBuilder, AsmCompiler};
    use sp1_recursion_executor::Runtime;

    use sp1_primitives::SP1Field;
    type F = SP1Field;
    type EF = BinomialExtensionField<SP1Field, 4>;

    #[tokio::test]
    async fn test_merkle_proof() {
        let mut rng = thread_rng();

        let height = rng.gen_range(500..2000);
        let width = rng.gen_range(15..30);
        let num_tensors = rng.gen_range(5..15);

        let num_indices = rng.gen_range(2..10);

        let tensors = (0..num_tensors)
            .map(|_| Tensor::<SP1Field>::rand(&mut rng, [height, width]))
            .collect::<Message<_>>();

        let prover = SP1MerkleTreeProver::default();
        let (root, data) = prover.commit_tensors(tensors.clone()).await.unwrap();

        let indices = (0..num_indices).map(|_| rng.gen_range(0..height)).collect_vec();
        let proof = prover.prove_openings_at_indices(data, &indices).await.unwrap();
        let openings = prover.compute_openings_at_indices(tensors, &indices).await;
        let opening: MerkleTreeOpening<SP1GlobalContext> =
            MerkleTreeOpening { values: openings, proof };

        let bit_len = height.next_power_of_two().ilog2();

        let mut builder = AsmBuilder::default();
        let mut witness_stream = Vec::new();

        let mut index_bits = Vec::new();
        for index in indices {
            let bits = (0..bit_len).map(|i| (index >> i) & 1 == 1).collect_vec();
            Witnessable::<AsmConfig>::write(&bits, &mut witness_stream);
            let bits = bits.read(&mut builder);
            index_bits.push(bits);
        }

        Witnessable::<AsmConfig>::write(&root, &mut witness_stream);
        let root = root.read(&mut builder);
        Witnessable::<AsmConfig>::write(&opening, &mut witness_stream);
        let opening = opening.read(&mut builder);

        RecursiveMerkleTreeTcs::<AsmConfig, SP1CoreJaggedConfig>::verify_tensor_openings(
            &mut builder,
            &root,
            &index_bits,
            &opening,
        );

        let block = builder.into_root_block();
        let mut compiler = AsmCompiler::default();
        let program = Arc::new(compiler.compile_inner(block).validate().unwrap());
        let mut runtime = Runtime::<F, EF, SP1DiffusionMatrix>::new(program.clone(), inner_perm());
        runtime.witness_stream = witness_stream.into();
        runtime.run().unwrap();
    }

    #[tokio::test]
    async fn test_invalid_merkle_proof() {
        let mut rng = thread_rng();

        let height = rng.gen_range(500..2000);
        let width = rng.gen_range(15..30);
        let num_tensors = rng.gen_range(5..15);

        let num_indices = rng.gen_range(2..10);

        let tensors = (0..num_tensors)
            .map(|_| Tensor::<SP1Field>::rand(&mut rng, [height, width]))
            .collect::<Message<_>>();

        let prover = SP1MerkleTreeProver::default();
        let (root, data) = prover.commit_tensors(tensors.clone()).await.unwrap();

        let indices = (0..num_indices).map(|_| rng.gen_range(0..height)).collect_vec();
        let proof = prover.prove_openings_at_indices(data, &indices).await.unwrap();
        let openings = prover.compute_openings_at_indices(tensors, &indices).await;
        let opening: MerkleTreeOpening<SP1GlobalContext> =
            MerkleTreeOpening { values: openings, proof };

        let bit_len = height.next_power_of_two().ilog2();

        let mut builder = AsmBuilder::default();
        let mut witness_stream = Vec::new();

        let mut index_bits = Vec::new();
        for index in indices {
            let bits = (0..bit_len)
                .map(|i| if i == 0 { (index >> i) & 1 == 0 } else { (index >> i) & 1 == 1 })
                .collect_vec();
            Witnessable::<AsmConfig>::write(&bits, &mut witness_stream);
            let bits = bits.read(&mut builder);
            index_bits.push(bits);
        }

        Witnessable::<AsmConfig>::write(&root, &mut witness_stream);
        let root = root.read(&mut builder);
        Witnessable::<AsmConfig>::write(&opening, &mut witness_stream);
        let opening = opening.read(&mut builder);

        RecursiveMerkleTreeTcs::<AsmConfig, SP1CoreJaggedConfig>::verify_tensor_openings(
            &mut builder,
            &root,
            &index_bits,
            &opening,
        );

        let block = builder.into_root_block();
        let mut compiler = AsmCompiler::default();
        let program = Arc::new(compiler.compile_inner(block).validate().unwrap());
        let mut runtime = Runtime::<F, EF, SP1DiffusionMatrix>::new(program.clone(), inner_perm());
        runtime.witness_stream = witness_stream.into();
        runtime.run().expect_err("merkle proof should not verify");
    }
}
