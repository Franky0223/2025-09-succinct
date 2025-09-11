#![allow(missing_docs)]

use crate::{cpu::CPUProvingKey, ProvingKey};
use sp1_cuda::CudaProvingKey;
use sp1_prover::SP1VerifyingKey;

#[derive(Clone)]
pub enum EnvProvingKey {
    Cpu {
        pk: CPUProvingKey,
        seal: sealed::Seal,
    },
    Cuda {
        pk: CudaProvingKey,
        seal: sealed::Seal,
    },
    Mock {
        pk: CPUProvingKey,
        seal: sealed::Seal,
    },
    #[cfg(feature = "network")]
    Network {
        pk: CPUProvingKey,
        seal: sealed::Seal,
    },
}

impl EnvProvingKey {
    pub(crate) const fn cpu(inner: CPUProvingKey) -> Self {
        Self::Cpu { pk: inner, seal: sealed::Seal::new() }
    }

    pub(crate) const fn cuda(inner: CudaProvingKey) -> Self {
        Self::Cuda { pk: inner, seal: sealed::Seal::new() }
    }

    pub(crate) const fn mock(inner: CPUProvingKey) -> Self {
        Self::Mock { pk: inner, seal: sealed::Seal::new() }
    }

    #[cfg(feature = "network")]
    pub(crate) const fn network(inner: CPUProvingKey) -> Self {
        Self::Network { pk: inner, seal: sealed::Seal::new() }
    }
}

impl ProvingKey for EnvProvingKey {
    fn verifying_key(&self) -> &SP1VerifyingKey {
        #[allow(clippy::match_same_arms)]
        match self {
            Self::Cpu { pk, .. } => pk.verifying_key(),
            Self::Cuda { pk, .. } => pk.verifying_key(),
            Self::Mock { pk, .. } => pk.verifying_key(),
            #[cfg(feature = "network")]
            Self::Network { pk, .. } => pk.verifying_key(),
        }
    }

    fn elf(&self) -> &[u8] {
        #[allow(clippy::match_same_arms)]
        match self {
            Self::Cpu { pk, .. } => pk.elf(),
            Self::Cuda { pk, .. } => pk.elf(),
            Self::Mock { pk, .. } => pk.elf(),
            #[cfg(feature = "network")]
            Self::Network { pk, .. } => pk.elf(),
        }
    }
}

/// A seal for disallowing direct construction of `EnvProver` proving key.
mod sealed {
    #[derive(Clone)]
    pub struct Seal {
        _private: (),
    }

    impl Seal {
        pub(crate) const fn new() -> Self {
            Self { _private: () }
        }
    }
}
