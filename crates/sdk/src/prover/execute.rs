use crate::StatusCode;

use super::Prover;
use sp1_core_executor::{ExecutionError, ExecutionReport, HookEnv, SP1ContextBuilder};
use sp1_core_machine::io::SP1Stdin;
use sp1_primitives::{io::SP1PublicValues, Elf};
use std::{
    future::{Future, IntoFuture},
    pin::Pin,
};

/// A request for executing a program.
pub struct ExecuteRequest<'a, P: Prover> {
    pub(crate) prover: &'a P,
    pub(crate) elf: Elf,
    pub(crate) stdin: SP1Stdin,
    pub(crate) context_builder: SP1ContextBuilder<'static>,
}

impl<'a, P: Prover> ExecuteRequest<'a, P> {
    pub(crate) fn new(prover: &'a P, elf: Elf, stdin: SP1Stdin) -> Self {
        Self { prover, elf, stdin, context_builder: SP1ContextBuilder::new() }
    }

    /// Add a executor [`sp1_core_executor::Hook`] into the context.
    ///
    /// # Arguments
    /// * `fd` - The file descriptor that triggers this execution hook.
    /// * `f` - The function to invoke when the hook is triggered.
    ///
    /// # Details
    /// Hooks may be invoked from within SP1 by writing to the specified file descriptor `fd`
    /// with [`sp1_zkvm::io::write`], returning a list of arbitrary data that may be read
    /// with successive calls to [`sp1_zkvm::io::read`].
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_sdk::{include_elf, Prover, ProverClient, SP1Stdin};
    ///
    /// let elf = &[1, 2, 3];
    /// let stdin = SP1Stdin::new();
    ///
    /// let client = ProverClient::builder().cpu().build();
    /// let builder = client
    ///     .execute(elf, &stdin)
    ///     .with_hook(1, |env, data| {
    ///         println!("Hook triggered with data: {:?}", data);
    ///         vec![vec![1, 2, 3]]
    ///     })
    ///     .run();
    /// ```
    #[must_use]
    pub fn with_hook(
        mut self,
        fd: u32,
        f: impl FnMut(HookEnv, &[u8]) -> Vec<Vec<u8>> + Send + Sync + 'static,
    ) -> Self {
        self.context_builder.hook(fd, f);
        self
    }

    /// Set the maximum number of cpu cycles to use for execution.
    ///
    /// # Arguments
    /// * `max_cycles` - The maximum number of cycles to use for execution.
    ///
    /// # Details
    /// If the cycle limit is exceeded, execution will fail with the
    /// [`sp1_core_executor::ExecutionError::ExceededCycleLimit`]. This is useful for preventing
    /// infinite loops in the and limiting the execution time of the program.
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_sdk::{include_elf, Prover, ProverClient, SP1Stdin};
    ///
    /// let elf = &[1, 2, 3];
    /// let stdin = SP1Stdin::new();
    ///
    /// let client = ProverClient::builder().cpu().build();
    /// let builder = client.execute(elf, &stdin).cycle_limit(1000000).run();
    /// ```
    #[must_use]
    pub fn cycle_limit(mut self, max_cycles: u64) -> Self {
        self.context_builder.max_cycles(max_cycles);
        self
    }

    /// Whether to enable deferred proof verification in the executor.
    ///
    /// # Arguments
    /// * `value` - Whether to enable deferred proof verification in the executor.
    ///
    /// # Details
    /// Default: `true`. If set to `false`, the executor will skip deferred proof verification.
    /// This is useful for reducing the execution time of the program and optimistically assuming
    /// that the deferred proofs are correct. Can also be used for mock proof setups that require
    /// verifying mock compressed proofs.
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_sdk::{include_elf, Prover, ProverClient, SP1Stdin};
    ///
    /// let elf = &[1, 2, 3];
    /// let stdin = SP1Stdin::new();
    ///
    /// let client = ProverClient::builder().cpu().build();
    /// let builder = client.execute(elf, &stdin).deferred_proof_verification(false).await;
    /// ```
    #[must_use]
    pub fn deferred_proof_verification(mut self, value: bool) -> Self {
        self.context_builder.set_deferred_proof_verification(value);
        self
    }

    /// Whether to enable gas calculation in the executor.
    ///
    /// # Arguments
    /// * `value` - Whether to enable gas calculation in the executor.
    ///
    /// # Details
    /// Default: `true`. If set to `false`, the executor will not calculate gas.
    /// This is useful for reducing the execution time of the program, since gas calculation
    /// must perform extra work to simulate parts of the proving process.
    ///
    /// Gas may be retrieved through the [`ExecutionReport`] available through [`Self::run`].
    /// It will be `None` if and only if this option is disabled.
    ///
    /// # Example
    /// ```rust,no_run
    /// use sp1_sdk::{include_elf, Prover, ProverClient, SP1Stdin};
    ///
    /// let elf = &[1, 2, 3];
    /// let stdin = SP1Stdin::new();
    ///
    /// let client = ProverClient::builder().cpu().build();
    /// let builder = client.execute(elf, &stdin).calculate_gas(false).run();
    /// ```
    #[must_use]
    pub fn calculate_gas(mut self, value: bool) -> Self {
        self.context_builder.calculate_gas(value);
        self
    }

    /// Set the expected exit code of the program.
    ///
    /// # Arguments
    /// * `code` - The expected exit code of the program.
    #[must_use]
    pub fn expected_exit_code(mut self, code: StatusCode) -> Self {
        self.context_builder.expected_exit_code(code);
        self
    }

    // todo!(n): workaround this
    // /// Override the default stdout of the guest program.
    // ///
    // /// # Example
    // /// ```rust,no_run
    // /// use sp1_sdk::{include_elf, Prover, ProverClient, SP1Stdin};
    // ///
    // /// let mut stdout = Vec::new();
    // ///
    // /// let elf = &[1, 2, 3];
    // /// let stdin = SP1Stdin::new();
    // ///
    // /// let client = ProverClient::builder().cpu().build();
    // /// client.execute(elf, &stdin).stdout(&mut stdout).run();
    // /// ```
    // #[must_use]
    // pub fn stdout<W: IoWriter>(mut self, writer: &'a mut W) -> Self {
    //     self.context_builder.stdout(writer);
    //     self
    // }

    // /// Override the default stdout of the guest program.
    // ///
    // /// # Example
    // /// ```rust,no_run
    // /// use sp1_sdk::{include_elf, Prover, ProverClient, SP1Stdin};
    // ///
    // /// let mut stderr = Vec::new();
    // ///
    // /// let elf = &[1, 2, 3];
    // /// let stdin = SP1Stdin::new();
    // ///
    // /// let client = ProverClient::builder().cpu().build();
    // /// client.execute(elf, &stdin).stderr(&mut stderr).run();
    // /// ```
    // #[must_use]
    // pub fn stderr<W: IoWriter>(mut self, writer: &'a mut W) -> Self {
    //     self.context_builder.stderr(writer);
    //     self
    // }
}

impl<'a, P: Prover> IntoFuture for ExecuteRequest<'a, P> {
    type Output = Result<(SP1PublicValues, ExecutionReport), ExecutionError>;

    type IntoFuture = Pin<Box<dyn Future<Output = Self::Output> + Send + 'a>>;

    fn into_future(self) -> Self::IntoFuture {
        let task = async move {
            let Self { prover, elf, stdin, mut context_builder } = self;
            let inner = prover.inner();
            let context = context_builder.build();

            let result_handle = tokio::task::spawn_blocking(move || {
                let (pv, _, report) = inner.execute(&elf, &stdin, context)?;
                Ok((pv, report))
            });

            // todo!(n): if there exists stdout/stderr pipes can just forward them with an mpsc
            // here, and then write to the actual stdout/stderr writers from this
            // future.

            result_handle.await.unwrap()
        };
        Box::pin(task)
    }
}
