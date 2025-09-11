use std::{
    mem::ManuallyDrop,
    ops::{Add, Deref, DerefMut},
};

use rayon::prelude::*;

use derive_where::derive_where;
use rand::{distributions::Standard, prelude::Distribution, Rng};
use slop_algebra::{AbstractExtensionField, AbstractField, Field};
use slop_alloc::{Backend, Buffer, CpuBackend, HasBackend, GLOBAL_CPU_BACKEND};
use slop_tensor::Tensor;

use crate::{
    eval_mle_at_point_blocking, eval_monomial_basis_mle_at_point_blocking,
    partial_lagrange_blocking, MleBaseBackend, MleEvaluationBackend, MleFixLastVariableBackend,
    MleFixLastVariableInPlaceBackend, MleFixedAtZeroBackend, MleFoldBackend,
    PartialLagrangeBackend, Point, ZeroEvalBackend,
};

pub enum Basis {
    Monomial,
    Evaluation,
}

/// A batch of multi-linear polynomials.
#[derive(Debug, Clone)]
#[derive_where(PartialEq, Eq, Serialize, Deserialize; Tensor<T, A>)]
pub struct Mle<T, A: Backend = CpuBackend> {
    guts: Tensor<T, A>,
}

impl<F, A: Backend> HasBackend for Mle<F, A> {
    type Backend = A;

    #[inline]
    fn backend(&self) -> &Self::Backend {
        self.guts.backend()
    }
}

impl<F, A: Backend> Mle<F, A> {
    /// Creates a new MLE from a tensor in the correct shape.
    ///
    /// The tensor must be in the correct shape for the given backend.
    #[inline]
    pub const fn new(guts: Tensor<F, A>) -> Self {
        Self { guts }
    }

    /// Creates a new MLE from a buffer, assumed to be a single polynomial.
    #[inline]
    pub fn from_buffer(buffer: Buffer<F, A>) -> Self
    where
        F: AbstractField,
        A: MleBaseBackend<F>,
    {
        // First, we need to convert the buffer into an arbitrary 2 dimensional tensor.
        let size = buffer.len();
        let mut tensor = Tensor::from(buffer).reshape([size, 1]);

        // Then, we need to convert the tensor into the correct shape, which is determined by the
        // backend.
        let dim_0 = A::num_polynomials(&tensor);
        let dim_1 = A::num_non_zero_entries(&tensor);
        tensor.reshape_in_place([dim_1, dim_0]);
        Self::new(tensor)
    }

    #[inline]
    pub fn backend(&self) -> &A {
        self.guts.backend()
    }

    #[inline]
    pub fn into_guts(self) -> Tensor<F, A> {
        self.guts
    }

    /// Creates a new uninitialized MLE batch of the given size and number of variables.
    #[inline]
    pub fn uninit(num_polynomials: usize, num_non_zero_entries: usize, scope: &A) -> Self
    where
        F: AbstractField,
        A: MleBaseBackend<F>,
    {
        // The tensor is initialized in the correct shape by the backend.
        Self::new(scope.uninit_mle(num_polynomials, num_non_zero_entries))
    }

    #[inline]
    pub const fn guts(&self) -> &Tensor<F, A> {
        &self.guts
    }

    /// Mutable access to the guts of the MLE.
    ///
    /// Changing the guts must preserve the layout that the MLE backend expects to have for a valid
    /// tensor to qualify as the guts of an MLE. For example, dimension matching the implementation
    /// of [Self::uninit].
    pub fn guts_mut(&mut self) -> &mut Tensor<F, A> {
        &mut self.guts
    }

    /// # Safety
    #[inline]
    pub unsafe fn assume_init(&mut self) {
        self.guts.assume_init();
    }

    /// Returns the number of polynomials in the batch.
    #[inline]
    pub fn num_polynomials(&self) -> usize
    where
        F: AbstractField,
        A: MleBaseBackend<F>,
    {
        A::num_polynomials(&self.guts)
    }

    /// Returns the number of variables in the polynomials.
    #[inline]
    pub fn num_variables(&self) -> u32
    where
        F: AbstractField,
        A: MleBaseBackend<F>,
    {
        A::num_variables(&self.guts)
    }

    /// Returns the number of points on the hypercube that are non-zero, with respect to the
    /// canonical ordering.
    #[inline]
    pub fn num_non_zero_entries(&self) -> usize
    where
        F: AbstractField,
        A: MleBaseBackend<F>,
    {
        A::num_non_zero_entries(&self.guts)
    }

    /// Computes the partial lagrange polynomial eq(z, -) for a fixed z.
    #[inline]
    pub async fn partial_lagrange(point: &Point<F, A>) -> Mle<F, A>
    where
        F: AbstractField,
        A: PartialLagrangeBackend<F>,
    {
        let guts = A::partial_lagrange(point).await;
        Mle::new(guts)
    }

    /// Evaluates the MLE at a given point.
    #[inline]
    pub async fn eval_at<EF: AbstractExtensionField<F>>(
        &self,
        point: &Point<EF, A>,
    ) -> MleEval<EF, A>
    where
        F: AbstractField,
        A: MleEvaluationBackend<F, EF>,
    {
        let evaluations = A::eval_mle_at_point(&self.guts, point).await;
        MleEval::new(evaluations)
    }

    /// Evaluates the MLE at a given eq.
    #[inline]
    pub async fn eval_at_eq<EF: AbstractExtensionField<F>>(&self, eq: &Mle<EF, A>) -> MleEval<EF, A>
    where
        F: AbstractField,
        A: MleEvaluationBackend<F, EF>,
    {
        let evaluations = A::eval_mle_at_eq(&self.guts, &eq.guts).await;
        MleEval::new(evaluations)
    }

    /// Compute the random linear combination of the even and odd coefficients of `vals`.
    ///
    /// This is used in the `Basefold` PCS.
    #[inline]
    pub async fn fold(&self, beta: F) -> Mle<F, A>
    where
        F: AbstractField,
        A: MleFoldBackend<F>,
    {
        let guts = A::fold_mle(&self.guts, beta).await;
        Mle::new(guts)
    }

    #[inline]
    pub async fn fix_last_variable<EF>(&self, alpha: EF) -> Mle<EF, A>
    where
        F: AbstractField,
        EF: AbstractExtensionField<F>,
        A: MleFixLastVariableBackend<F, EF>,
    {
        let guts = A::mle_fix_last_variable_constant_padding(&self.guts, alpha, F::zero()).await;
        Mle::new(guts)
    }

    #[inline]
    pub async fn fix_last_variable_in_place(&mut self, alpha: F)
    where
        F: AbstractField,
        A: MleFixLastVariableInPlaceBackend<F>,
    {
        A::mle_fix_last_variable_in_place(&mut self.guts, alpha).await;
    }

    #[inline]
    pub async fn fixed_at_zero<EF: AbstractExtensionField<F>>(
        &self,
        point: &Point<EF>,
    ) -> MleEval<EF>
    where
        F: AbstractField,
        A: MleFixedAtZeroBackend<F, EF>,
    {
        MleEval::new(A::fixed_at_zero(&self.guts, point).await)
    }

    /// # Safety
    ///
    /// This function is unsafe because it enables bypassing the lifetime of the mle.
    #[inline]
    pub unsafe fn owned_unchecked(&self) -> ManuallyDrop<Self> {
        self.owned_unchecked_in(self.guts.storage.allocator().clone())
    }

    /// # Safety
    ///
    /// This function is unsafe because it enables bypassing the lifetime of the mle.
    #[inline]
    pub unsafe fn owned_unchecked_in(&self, storage_allocator: A) -> ManuallyDrop<Self> {
        let guts = self.guts.owned_unchecked_in(storage_allocator);
        let guts = ManuallyDrop::into_inner(guts);
        ManuallyDrop::new(Self { guts })
    }
}

impl<T> Mle<T, CpuBackend> {
    pub fn rand<R: Rng>(rng: &mut R, num_polynomials: usize, num_variables: u32) -> Self
    where
        Standard: Distribution<T>,
    {
        Self::new(Tensor::rand(rng, [1 << num_variables, num_polynomials]))
    }

    /// Returns an iterator over the evaluations of the MLE on the Boolean hypercube.
    ///
    /// The iterator yields a slice for each index of the Boolean hypercube.
    pub fn hypercube_iter(&self) -> impl Iterator<Item = &[T]>
    where
        T: AbstractField,
    {
        let width = self.num_polynomials();
        let height = self.num_variables();
        (0..(1 << height)).map(move |i| &self.guts.as_slice()[i * width..(i + 1) * width])
    }

    /// Returns an iterator over the evaluations of the MLE on the Boolean hypercube.
    ///
    /// The iterator yields a slice for each index of the Boolean hypercube.
    pub fn hypercube_par_iter(&self) -> impl IndexedParallelIterator<Item = &[T]>
    where
        T: AbstractField + Sync,
    {
        let width = self.num_polynomials();
        let height = self.num_variables();
        (0..(1 << height))
            .into_par_iter()
            .map(move |i| &self.guts.as_slice()[i * width..(i + 1) * width])
    }

    /// # Safety
    pub unsafe fn from_raw_parts(ptr: *mut T, num_polynomials: usize, len: usize) -> Self {
        let total_len = num_polynomials * len;
        let buffer = Buffer::from_raw_parts(ptr, total_len, total_len, GLOBAL_CPU_BACKEND);
        Self::new(Tensor::from(buffer).reshape([len, num_polynomials]))
    }

    /// Evaluate the `Mle` at `point` assuming that the guts of the `Mle` is the set of evaluations
    /// of the `Mle` on the Boolean hypercube.
    pub fn blocking_eval_at<E>(&self, point: &Point<E>) -> MleEval<E>
    where
        T: AbstractField + 'static + Send + Sync,
        E: AbstractExtensionField<T> + 'static + Send + Sync,
    {
        MleEval::new(eval_mle_at_point_blocking(self.guts(), point))
    }

    /// Evaluate the `Mle` at `point` assuming that the entry at index `i = (i_0,...,i_{n-1})` is the
    /// coefficient of the monomial `X_0^{i_0} ... X_{n-1}^{i_{n-1}}`, where `i_0` is the most
    /// significant bit of `i` and `i_{n-1}` is the least-significant one.
    pub fn blocking_monomial_basis_eval_at<E>(&self, point: &Point<E>) -> MleEval<E>
    where
        T: AbstractField + 'static + Send + Sync,
        E: AbstractExtensionField<T> + 'static + Send + Sync,
    {
        MleEval::new(eval_monomial_basis_mle_at_point_blocking(self.guts(), point))
    }

    pub fn blocking_partial_lagrange(point: &Point<T>) -> Mle<T, CpuBackend>
    where
        T: 'static + AbstractField,
    {
        let guts = partial_lagrange_blocking(point);
        Mle::new(guts)
    }

    /// Evaluates the 2n-variate multilinear polynomial f(X,Y) = Prod_i (X_i * Y_i + (1-X_i) * (1-Y_i))
    /// at a given pair (X,Y) of n-dimenional BabyBearExtensionField points.
    ///
    /// This evaluation takes time linear in n to compute, so the verifier can easily compute it. Hence,
    /// even though
    /// ```full_lagrange_eval(point_1, point_2)==partial_lagrange_eval(point_1).eval_at_point(point_2)```,
    /// the RHS of the above equation runs in O(2^n) time, while the LHS runs in O(n).
    ///
    /// The polynomial f(X,Y) is an important building block in zerocheck and other protocols which use
    /// sumcheck.
    pub fn full_lagrange_eval<EF>(point_1: &Point<T>, point_2: &Point<EF>) -> EF
    where
        T: AbstractField,
        EF: AbstractExtensionField<T>,
    {
        assert_eq!(point_1.dimension(), point_2.dimension());

        // Iterate over all values in the n-variates X and Y.
        point_1
            .iter()
            .zip(point_2.iter())
            .map(|(x, y)| {
                // Multiply by (x_i * y_i + (1-x_i) * (1-y_i)).
                let prod = y.clone() * x.clone();
                prod.clone() + prod + EF::one() - x.clone() - y.clone()
            })
            .product()
    }

    /// The analogue of `full_lagrange_eval` for the monomial basis.
    pub fn full_monomial_basis_eq<EF>(point_1: &Point<T>, point_2: &Point<EF>) -> EF
    where
        T: AbstractField,
        EF: AbstractExtensionField<T>,
    {
        assert_eq!(point_1.dimension(), point_2.dimension());

        // Iterate over all values in the n-variates X and Y.
        point_1
            .iter()
            .zip(point_2.iter())
            .map(|(x, y)| {
                // Multiply by (x_i * y_i + (1-y_i)).
                let prod = y.clone() * x.clone();
                prod + EF::one() - y.clone()
            })
            .product()
    }
}

impl<T: AbstractField + Send + Sync> TryInto<slop_matrix::dense::RowMajorMatrix<T>>
    for Mle<T, CpuBackend>
{
    type Error = ();

    fn try_into(self) -> Result<slop_matrix::dense::RowMajorMatrix<T>, Self::Error> {
        let num_polys = self.num_polynomials();
        let values = self.guts.into_buffer().to_vec();
        Ok(slop_matrix::dense::RowMajorMatrix::new(values, num_polys))
    }
}

impl<T> From<Vec<T>> for Mle<T, CpuBackend> {
    fn from(values: Vec<T>) -> Self {
        let len = values.len();
        let tensor = Tensor::from(values).reshape([len, 1]);
        Self::new(tensor)
    }
}

impl<T: Clone + Send + Sync> From<slop_matrix::dense::RowMajorMatrix<T>> for Mle<T, CpuBackend> {
    fn from(values: slop_matrix::dense::RowMajorMatrix<T>) -> Self {
        Self::new(Tensor::from(values))
    }
}

impl<T> FromIterator<T> for Mle<T, CpuBackend> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<Vec<_>>())
    }
}

/// The multilinear polynomial whose evaluation on the Boolean hypercube performs outputs 1 if the
/// Boolean hypercube point is the bit-string representation of a number greater than or equal to
/// `threshold`, and 0 otherwise.
pub fn partial_geq<F: Field>(threshold: usize, num_variables: usize) -> Vec<F> {
    assert!(threshold <= 1 << num_variables);

    (0..(1 << num_variables)).map(|x| if x >= threshold { F::one() } else { F::zero() }).collect()
}

/// A succinct way to compute the evaluation of `partial_geq` at `eval_point`. The threshold is passed
/// as a `Point` on the Boolean hypercube.
///
/// # Panics
/// If the dimensions of `threshold` and `eval_point` do not match.
pub fn full_geq<F: AbstractField, EF: AbstractExtensionField<F>>(
    threshold: &Point<F>,
    eval_point: &Point<EF>,
) -> EF {
    assert_eq!(threshold.dimension(), eval_point.dimension());
    threshold.iter().rev().zip(eval_point.iter().rev()).fold(EF::one(), |acc, (x, y)| {
        ((EF::one() - y.clone()) * (F::one() - x.clone()) + y.clone() * x.clone()) * acc
            + y.clone() * (F::one() - x.clone())
    })
}

/// A bacth of multi-linear polynomial evaluations.
#[derive(Debug, Clone)]
#[derive_where(PartialEq, Eq, Serialize, Deserialize; Tensor<T, A>)]
pub struct MleEval<T, A: Backend = CpuBackend> {
    pub(crate) evaluations: Tensor<T, A>,
}

impl<T, A: Backend> MleEval<T, A> {
    /// Creates a new MLE evaluation from a tensor in the correct shape.
    #[inline]
    pub const fn new(evaluations: Tensor<T, A>) -> Self {
        Self { evaluations }
    }

    #[inline]
    pub fn evaluations(&self) -> &Tensor<T, A> {
        &self.evaluations
    }

    #[inline]
    pub fn zeros_in(num_polynomials: usize, allocator: &A) -> MleEval<T, A>
    where
        T: AbstractField,
        A: ZeroEvalBackend<T>,
    {
        MleEval::new(allocator.zero_evaluations(num_polynomials))
    }

    /// # Safety
    #[inline]
    pub unsafe fn evaluations_mut(&mut self) -> &mut Tensor<T, A> {
        &mut self.evaluations
    }

    #[inline]
    pub fn into_evaluations(self) -> Tensor<T, A> {
        self.evaluations
    }

    //. It is expected that `self.evaluations.sizes()` is one of the three options:
    /// `[1, num_polynomials]`, `[num_polynomials,1]`, or `[num_polynomials]`.
    #[inline]
    pub fn num_polynomials(&self) -> usize {
        self.evaluations.total_len()
    }

    /// # Safety
    ///
    /// This function is unsafe because it enables bypassing the lifetime of the mle.
    #[inline]
    pub unsafe fn owned_unchecked_in(&self, storage_allocator: A) -> ManuallyDrop<Self> {
        let evaluations = self.evaluations.owned_unchecked_in(storage_allocator);
        let evaluations = ManuallyDrop::into_inner(evaluations);
        ManuallyDrop::new(Self { evaluations })
    }

    /// # Safety
    ///
    /// This function is unsafe because it enables bypassing the lifetime of the mle.
    #[inline]
    pub unsafe fn owned_unchecked(&self) -> ManuallyDrop<Self> {
        self.owned_unchecked_in(self.evaluations.backend().clone())
    }
}

impl<T> MleEval<T, CpuBackend> {
    pub fn to_vec(&self) -> Vec<T>
    where
        T: Clone,
    {
        self.evaluations.as_buffer().to_vec()
    }

    pub fn iter(&self) -> impl Iterator<Item = &[T]> + '_ {
        self.evaluations.split().map(|t| t.as_slice())
    }

    pub fn zeros(num_polynomials: usize) -> MleEval<T, CpuBackend>
    where
        T: AbstractField,
    {
        MleEval::zeros_in(num_polynomials, &GLOBAL_CPU_BACKEND)
    }

    pub fn add_evals(self, other: Self) -> Self
    where
        T: Add<Output = T> + Clone,
    {
        self.to_vec().into_iter().zip(other.to_vec()).map(|(a, b)| a + b).collect::<Vec<_>>().into()
    }
}

impl<T> From<Vec<T>> for MleEval<T, CpuBackend> {
    fn from(evaluations: Vec<T>) -> Self {
        Self::new(evaluations.into())
    }
}

impl<T> Deref for MleEval<T, CpuBackend> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.evaluations.as_slice()
    }
}

impl<T> DerefMut for MleEval<T, CpuBackend> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.evaluations.as_mut_slice()
    }
}

impl<T, A: Backend> HasBackend for MleEval<T, A> {
    type Backend = A;

    fn backend(&self) -> &Self::Backend {
        self.evaluations.backend()
    }
}

impl<T> IntoIterator for MleEval<T, CpuBackend> {
    type Item = T;
    type IntoIter = <Vec<T> as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.evaluations.into_buffer().into_vec().into_iter()
    }
}

impl<'a, T> IntoIterator for &'a MleEval<T, CpuBackend> {
    type Item = &'a T;
    type IntoIter = <&'a [T] as IntoIterator>::IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        self.evaluations.as_slice().iter()
    }
}

impl<T> FromIterator<T> for MleEval<T, CpuBackend> {
    fn from_iter<I: IntoIterator<Item = T>>(iter: I) -> Self {
        Self::new(Tensor::from(iter.into_iter().collect::<Vec<_>>()))
    }
}

#[cfg(test)]
mod tests {

    use slop_algebra::extension::BinomialExtensionField;
    use slop_alloc::Buffer;
    use slop_baby_bear::BabyBear;

    use super::*;

    use crate::{full_geq, partial_geq, Mle};

    #[tokio::test]
    async fn test_mle_eval() {
        let mut rng = rand::thread_rng();

        type F = BabyBear;
        type EF = BinomialExtensionField<BabyBear, 4>;

        let num_variables = 11;
        let num_polynomials = 10;

        let mle = Mle::<F>::rand(&mut rng, num_polynomials, num_variables);

        // Test the correctness of values on the hypercube.
        for i in 0usize..(1 << num_variables) {
            // Get the big Endian bits of the index.
            let bits = (0..num_variables)
                .rev()
                .map(|j| (i >> j) & 1)
                .map(F::from_canonical_usize)
                .collect::<Vec<_>>();
            let point = Point::<F>::new(Buffer::from(bits));
            let value = mle.eval_at(&point).await.to_vec();
            for (j, v) in value.iter().enumerate() {
                assert_eq!(*mle.guts[[i, j]], *v);
            }
        }

        // Test the multi-linearity of evaluation.
        let point = Point::<EF>::rand(&mut rng, num_variables);

        let eval = mle.eval_at(&point).await;
        for i in 0..num_variables {
            let mut point_0 = point.clone();
            let mut point_1 = point.clone();
            let point_0_i_val: &mut EF = &mut point_0[i as usize];
            *point_0_i_val = EF::zero();
            let point_1_i_val: &mut EF = &mut point_1[i as usize];
            *point_1_i_val = EF::one();

            let eval_0 = mle.eval_at(&point_0);
            let eval_1 = mle.eval_at(&point_1);

            let z: EF = *point[i as usize];

            for ((eval_0, eval_1), eval) in eval_0
                .await
                .to_vec()
                .iter()
                .zip(eval_1.await.to_vec().iter())
                .zip(eval.to_vec().iter())
            {
                assert_eq!(*eval, *eval_0 * (EF::one() - z) + *eval_1 * z);
            }
        }

        // // Test the linearity of evaluation.
        // let rhs = Mle::<F>::rand(&mut rng, num_polynomials, num_variables);
        // let point = Point::<EF>::rand(&mut rng, num_variables);

        // let lhs_eval = mle.eval_at(&point);
        // let rhs_eval = rhs.eval_at(&point);
        // let sum_eval = (&mle + &rhs).eval_at(&point);

        // let lhs_eval_values = lhs_eval.to_vec();
        // let rhs_eval_values = rhs_eval.to_vec();
        // let sum_eval_values = sum_eval.to_vec();

        // for ((lhs, rhs), sum) in
        //     lhs_eval_values.iter().zip(rhs_eval_values.iter()).zip(sum_eval_values.iter())
        // {
        //     assert_eq!(*lhs + *rhs, *sum);
        // }
    }

    #[tokio::test]
    async fn test_mle_fold() {
        let mut rng = rand::thread_rng();

        type EF = BinomialExtensionField<BabyBear, 4>;

        let mle = Mle::<EF>::rand(&mut rng, 1, 11);
        let point = Point::<EF>::rand(&mut rng, 10);

        let beta = rng.gen::<EF>();

        let fold = mle.fold(beta).await;

        let mut point_0 = point.to_vec();
        point_0.push(EF::zero());
        let point_0 = Point::<EF>::from(point_0);

        let mut point_1 = point.to_vec();
        point_1.push(EF::one());
        let point_1 = Point::<EF>::from(point_1);

        let eval_0 = *mle.eval_at(&point_0).await.evaluations()[[0]];
        let eval_1 = *mle.eval_at(&point_1).await.evaluations()[[0]];
        let fold_eval = *fold.eval_at(&point).await.evaluations()[[0]];

        assert_eq!(fold_eval, eval_0 + eval_1 * beta);
    }

    #[tokio::test]
    pub async fn test_geq_polynomial() {
        let num_variables = 12;
        let mut rng = rand::thread_rng();

        type F = BabyBear;

        for threshold in 0..(1 << num_variables) {
            let eval_point =
                Point::<F>::from((0..num_variables).map(|_| rng.gen::<F>()).collect::<Vec<_>>());
            let geq_mle = Mle::from(partial_geq::<F>(threshold, num_variables));
            assert_eq!(
                geq_mle.eval_at(&eval_point).await.to_vec()[0],
                full_geq(&Point::from_usize(threshold, num_variables), &eval_point)
            );
        }
    }

    #[tokio::test]
    async fn test_mle_fix_last_variable() {
        let mut rng = rand::thread_rng();

        type EF = BinomialExtensionField<BabyBear, 4>;

        let num_polynomials = 5;
        let num_variables = 11;
        let mle = Mle::<EF>::rand(&mut rng, num_polynomials, num_variables);
        let alpha = rng.gen::<EF>();

        let fixed = mle.fix_last_variable(alpha).await;

        let mut point = Point::<EF>::rand(&mut rng, num_variables - 1);
        let fixed_eval = fixed.eval_at(&point).await;
        point.add_dimension_back(alpha);
        let mle_eval = mle.eval_at(&point).await;

        assert_eq!(fixed_eval.to_vec(), mle_eval.to_vec());
    }

    #[test]
    fn test_mle_serialization() {
        let mut rng = rand::thread_rng();

        type F = BabyBear;

        let mle = Mle::<F>::rand(&mut rng, 5, 11);

        let serialized = serde_json::to_string(&mle).unwrap();
        let deserialized: Mle<F> = serde_json::from_str(&serialized).unwrap();

        assert_eq!(mle, deserialized);
    }

    #[tokio::test]
    async fn test_blocking_mle_eval_at() {
        let mut rng = rand::thread_rng();

        type EF = BinomialExtensionField<BabyBear, 4>;

        let mle = Mle::<EF>::rand(&mut rng, 5, 11);
        let point = Point::<EF>::rand(&mut rng, 11);

        let eval = mle.eval_at(&point).await;
        let eval_blocking = mle.blocking_eval_at(&point);
        assert_eq!(eval.to_vec(), eval_blocking.to_vec());
    }
}
