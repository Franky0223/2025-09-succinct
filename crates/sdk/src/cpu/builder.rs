//! # CPU Prover Builder
//!
//! This module provides a builder for the [`CpuProver`].

use super::CpuProver;
use sp1_core_executor::SP1CoreOpts;

/// A builder for the [`CpuProver`].
///
/// The builder is used to configure the [`CpuProver`] before it is built.
pub struct CpuProverBuilder {
    /// Optional core options to configure the prover.
    core_opts: Option<SP1CoreOpts>,
}

impl Default for CpuProverBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuProverBuilder {
    /// Creates a new [`CpuProverBuilder`] with default settings.
    #[must_use]
    pub const fn new() -> Self {
        Self { core_opts: None }
    }

    /// Sets the core options for the prover.
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_core_executor::SP1CoreOpts;
    /// use sp1_sdk::ProverClient;
    ///
    /// let mut opts = SP1CoreOpts::default();
    /// opts.page_protect = true;
    /// let prover = ProverClient::builder().cpu().core_opts(opts).build().await;
    /// ```
    #[must_use]
    pub fn core_opts(mut self, opts: SP1CoreOpts) -> Self {
        self.core_opts = Some(opts);
        self
    }

    /// Sets the core options for the prover (alias for `core_opts`).
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_core_executor::SP1CoreOpts;
    /// use sp1_sdk::ProverClient;
    ///
    /// let mut opts = SP1CoreOpts::default();
    /// opts.page_protect = true;
    /// let prover = ProverClient::builder().cpu().with_opts(opts).build().await;
    /// ```
    #[must_use]
    pub fn with_opts(self, opts: SP1CoreOpts) -> Self {
        self.core_opts(opts)
    }

    /// Builds a [`CpuProver`].
    ///
    /// # Details
    /// This method will build a [`CpuProver`] with the given parameters. In particular, it will
    /// build a mock prover if the `mock` flag is set.
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_sdk::ProverClient;
    ///
    /// let prover = ProverClient::builder().cpu().build().await;
    /// ```
    #[must_use]
    pub async fn build(self) -> CpuProver {
        CpuProver::new_with_opts(self.core_opts).await
    }
}
