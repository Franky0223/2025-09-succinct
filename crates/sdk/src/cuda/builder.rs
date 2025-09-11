//! # CUDA Prover Builder
//!
//! This module provides a builder for the [`CudaProver`].

use super::CudaProver;
use crate::cpu::CpuProver;
use sp1_core_executor::SP1CoreOpts;
use sp1_cuda::CudaProver as CudaProverImpl;

/// A builder for the [`CudaProver`].
///
/// The builder is used to configure the [`CudaProver`] before it is built.
#[derive(Debug, Default)]
pub struct CudaProverBuilder {
    cuda_device_id: Option<u32>,
    /// Optional core options to configure the underlying CPU prover.
    core_opts: Option<SP1CoreOpts>,
}

impl CudaProverBuilder {
    /// Sets the CUDA device id.
    ///
    /// # Details
    /// Run the CUDA prover with the provided device id, all operations will be performed on this
    /// device index.
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_sdk::ProverClient;
    ///
    /// let prover = ProverClient::builder().cuda().with_device_id(0).build();
    /// ```
    #[must_use]
    pub fn with_device_id(mut self, id: u32) -> Self {
        self.cuda_device_id = Some(id);
        self
    }

    /// Sets the core options for the underlying CPU prover.
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_core_executor::SP1CoreOpts;
    /// use sp1_sdk::ProverClient;
    ///
    /// let mut opts = SP1CoreOpts::default();
    /// opts.page_protect = true;
    /// let prover = ProverClient::builder().cuda().core_opts(opts).build().await;
    /// ```
    #[must_use]
    pub fn core_opts(mut self, opts: SP1CoreOpts) -> Self {
        self.core_opts = Some(opts);
        self
    }

    /// Sets the core options for the underlying CPU prover (alias for `core_opts`).
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_core_executor::SP1CoreOpts;
    /// use sp1_sdk::ProverClient;
    ///
    /// let mut opts = SP1CoreOpts::default();
    /// opts.page_protect = true;
    /// let prover = ProverClient::builder().cuda().with_opts(opts).build().await;
    /// ```
    #[must_use]
    pub fn with_opts(self, opts: SP1CoreOpts) -> Self {
        self.core_opts(opts)
    }

    /// Builds a [`CudaProver`].
    ///
    /// # Details
    /// This method will build a [`CudaProver`] with the given parameters.
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_sdk::ProverClient;
    ///
    /// let prover = ProverClient::builder().cuda().build();
    /// ```
    #[must_use]
    pub async fn build(self) -> CudaProver {
        let cpu_prover = CpuProver::new_with_opts(self.core_opts).await;
        let cuda_prover = match self.cuda_device_id {
            Some(id) => CudaProverImpl::new_with_id(id).await,
            None => CudaProverImpl::new().await,
        };

        CudaProver {
            cpu_prover,
            prover: cuda_prover.expect("Failed to create the CUDA prover impl"),
        }
    }
}
