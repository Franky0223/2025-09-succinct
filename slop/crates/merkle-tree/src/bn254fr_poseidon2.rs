use std::marker::PhantomData;

use ff::PrimeField as FFPrimeField;
use serde::{Deserialize, Serialize};
use slop_algebra::{ExtensionField, PrimeField31};
use zkhash::{
    ark_ff::{BigInteger, PrimeField},
    fields::bn256::FpBN256 as ark_FpBN256,
    poseidon2::poseidon2_instance_bn256::RC3,
};

use slop_bn254::{
    Bn254Fr, DiffusionMatrixBN254, FFBn254Fr, OuterPerm, Poseidon2Bn254GlobalConfig,
    OUTER_CHALLENGER_STATE_WIDTH, OUTER_DIGEST_SIZE,
};
use slop_poseidon2::Poseidon2ExternalMatrixGeneral;
use slop_symmetric::{MultiField32PaddingFreeSponge, TruncatedPermutation};

use crate::{DefaultMerkleTreeConfig, MerkleTreeConfig};

#[derive(Debug, Clone, Default, Copy, Serialize, Deserialize, Hash, PartialEq, Eq)]
pub struct Poseidon2Bn254Config<F>(PhantomData<F>);

impl<F: PrimeField31, EF: ExtensionField<F>> MerkleTreeConfig<Poseidon2Bn254GlobalConfig<F, EF>>
    for Poseidon2Bn254Config<F>
{
    type Hasher = MultiField32PaddingFreeSponge<
        F,
        Bn254Fr,
        OuterPerm,
        OUTER_CHALLENGER_STATE_WIDTH,
        16,
        OUTER_DIGEST_SIZE,
    >;
    type Compressor =
        TruncatedPermutation<OuterPerm, 2, OUTER_DIGEST_SIZE, OUTER_CHALLENGER_STATE_WIDTH>;
}

impl<F: PrimeField31, EF: ExtensionField<F>>
    DefaultMerkleTreeConfig<Poseidon2Bn254GlobalConfig<F, EF>> for Poseidon2Bn254Config<F>
{
    fn default_hasher_and_compressor() -> (Self::Hasher, Self::Compressor) {
        let perm = outer_perm();
        let hasher = Self::Hasher::new(perm.clone()).unwrap();
        let compressor = Self::Compressor::new(perm.clone());
        (hasher, compressor)
    }
}

pub fn outer_perm() -> OuterPerm {
    const ROUNDS_F: usize = 8;
    const ROUNDS_P: usize = 56;
    let mut round_constants = bn254_poseidon2_rc3();
    let internal_start = ROUNDS_F / 2;
    let internal_end = (ROUNDS_F / 2) + ROUNDS_P;
    let internal_round_constants =
        round_constants.drain(internal_start..internal_end).map(|vec| vec[0]).collect::<Vec<_>>();
    let external_round_constants = round_constants;
    OuterPerm::new(
        ROUNDS_F,
        external_round_constants,
        Poseidon2ExternalMatrixGeneral,
        ROUNDS_P,
        internal_round_constants,
        DiffusionMatrixBN254,
    )
}

fn bn254_from_ark_ff(input: ark_FpBN256) -> Bn254Fr {
    let bytes = input.into_bigint().to_bytes_le();

    let mut res = <FFBn254Fr as ff::PrimeField>::Repr::default();

    for (i, digit) in res.0.as_mut().iter_mut().enumerate() {
        *digit = bytes[i];
    }

    let value = FFBn254Fr::from_repr(res);

    if value.is_some().into() {
        Bn254Fr { value: value.unwrap() }
    } else {
        panic!("Invalid field element")
    }
}

pub fn bn254_poseidon2_rc3() -> Vec<[Bn254Fr; 3]> {
    RC3.iter()
        .map(|vec| {
            vec.iter().cloned().map(bn254_from_ark_ff).collect::<Vec<_>>().try_into().unwrap()
        })
        .collect()
}
