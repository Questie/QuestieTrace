//! L2-regularized logistic regression, fitted with Newton's method. Feature counts are small (a
//! few per signal), so a dense solve per iteration is instant and needs no dependency.

#[derive(Clone, Debug, PartialEq)]
pub struct LogisticModel {
    pub intercept: f64,
    pub weights: Vec<f64>,
}

impl LogisticModel {
    pub fn zero(width: usize) -> LogisticModel {
        LogisticModel {
            intercept: 0.0,
            weights: vec![0.0; width],
        }
    }

    pub fn predict(&self, x: &[f64]) -> f64 {
        let mut z = self.intercept;
        for (weight, value) in self.weights.iter().zip(x) {
            z += weight * value;
        }
        sigmoid(z)
    }
}

pub struct FitOptions<'p> {
    /// Ridge strength on the weights.
    pub ridge: f64,
    /// Ridge on the intercept. Tiny: it only stops divergence when every label is equal.
    pub intercept_ridge: f64,
    /// Center of the ridge, which also fixes the width when there are no rows.
    pub prior: &'p LogisticModel,
    pub max_iterations: usize,
}

impl<'p> FitOptions<'p> {
    pub fn new(ridge: f64, prior: &'p LogisticModel) -> FitOptions<'p> {
        FitOptions {
            ridge,
            intercept_ridge: 1e-3,
            prior,
            max_iterations: 50,
        }
    }
}

fn sigmoid(z: f64) -> f64 {
    if z >= 0.0 {
        1.0 / (1.0 + (-z).exp())
    } else {
        z.exp() / (1.0 + z.exp())
    }
}

pub fn logit(p: f64) -> f64 {
    (p / (1.0 - p)).ln()
}

/// Fits `labels` (0 or 1) on `rows`, which all have the prior's width.
pub fn fit_logistic<R: AsRef<[f64]>>(rows: &[R], labels: &[f64], options: &FitOptions) -> LogisticModel {
    let prior = options.prior;
    let width = prior.weights.len();
    let size = width + 1;
    // beta[0] is the intercept, beta[1..] the weights.
    let center: Vec<f64> = std::iter::once(prior.intercept).chain(prior.weights.iter().copied()).collect();
    let mut beta = center.clone();
    let ridge: Vec<f64> = std::iter::once(options.intercept_ridge)
        .chain(std::iter::repeat_n(options.ridge, width))
        .collect();
    // Rows are sparse (a handful of signals claim any one edge), so only their non-zero columns are visited.
    let sparse: Vec<Vec<(usize, f64)>> = rows
        .iter()
        .map(|row| {
            let row = row.as_ref();
            assert_eq!(row.len(), width, "a row has another width than the prior");
            std::iter::once((0, 1.0))
                .chain(row.iter().enumerate().filter(|(_, &value)| value != 0.0).map(|(j, &value)| (j + 1, value)))
                .collect()
        })
        .collect();

    for _ in 0..options.max_iterations {
        let mut gradient: Vec<f64> = (0..size).map(|j| ridge[j] * (beta[j] - center[j])).collect();
        let mut hessian = vec![vec![0.0; size]; size];
        for j in 0..size {
            hessian[j][j] = ridge[j];
        }
        for (row, &label) in sparse.iter().zip(labels) {
            let mut z = 0.0;
            for &(column, value) in row {
                z += beta[column] * value;
            }
            let p = sigmoid(z);
            let curvature = p * (1.0 - p);
            for &(a, value_a) in row {
                gradient[a] += (p - label) * value_a;
                for &(b, value_b) in row {
                    hessian[a][b] += curvature * value_a * value_b;
                }
            }
        }
        let step = solve(&hessian, &gradient);
        let mut largest: f64 = 0.0;
        for j in 0..size {
            beta[j] -= step[j];
            largest = largest.max(step[j].abs());
        }
        if largest < 1e-7 {
            break;
        }
    }
    LogisticModel {
        intercept: beta[0],
        weights: beta[1..].to_vec(),
    }
}

/// Solves A x = b for a symmetric positive definite A (Cholesky).
fn solve(a: &[Vec<f64>], b: &[f64]) -> Vec<f64> {
    let n = b.len();
    let mut l = vec![vec![0.0; n]; n];
    for i in 0..n {
        for j in 0..=i {
            let sum = l[i][..j].iter().zip(&l[j][..j]).fold(a[i][j], |sum, (lik, ljk)| sum - lik * ljk);
            l[i][j] = if i == j { sum.max(1e-12).sqrt() } else { sum / l[j][j] };
        }
    }
    let mut y = vec![0.0; n];
    for i in 0..n {
        let mut sum = b[i];
        for k in 0..i {
            sum -= l[i][k] * y[k];
        }
        y[i] = sum / l[i][i];
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let mut sum = y[i];
        for k in i + 1..n {
            sum -= l[k][i] * x[k];
        }
        x[i] = sum / l[i][i];
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    /// mulberry32: a seeded generator, so the sample (and the test) never changes.
    fn random(mut seed: u32) -> impl FnMut() -> f64 {
        move || {
            seed = seed.wrapping_add(0x6d2b_79f5);
            let mut t = (seed ^ (seed >> 15)).wrapping_mul(1 | seed);
            t = (t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t))) ^ t;
            f64::from(t ^ (t >> 14)) / 4_294_967_296.0
        }
    }

    #[test]
    fn recovers_the_weights_that_generated_the_labels() {
        let mut next = random(7);
        let truth = LogisticModel {
            intercept: -0.5,
            weights: vec![2.0, -1.0],
        };
        let rows: Vec<Vec<f64>> = (0..5000).map(|_| vec![if next() < 0.5 { 1.0 } else { 0.0 }, next() * 4.0 - 2.0]).collect();
        let labels: Vec<f64> = rows.iter().map(|x| if next() < truth.predict(x) { 1.0 } else { 0.0 }).collect();

        let model = fit_logistic(&rows, &labels, &FitOptions::new(1e-6, &LogisticModel::zero(2)));

        assert!((model.intercept - truth.intercept).abs() < 0.5, "{model:?}");
        assert!((model.weights[0] - truth.weights[0]).abs() < 0.5, "{model:?}");
        assert!((model.weights[1] - truth.weights[1]).abs() < 0.5, "{model:?}");
    }

    #[test]
    fn shrinks_toward_the_prior_not_toward_zero() {
        let prior = LogisticModel {
            intercept: 0.3,
            weights: vec![0.0, 3.0],
        };
        // The second column is never set, so the data says nothing about it and the prior stands.
        let rows: Vec<Vec<f64>> = (0..200).map(|i| vec![f64::from(i % 2), 0.0]).collect();
        let labels: Vec<f64> = (0..200).map(|i| if i % 4 == 0 { 1.0 } else { 0.0 }).collect();

        let model = fit_logistic(&rows, &labels, &FitOptions::new(1.0, &prior));

        assert!((model.weights[1] - 3.0).abs() < 1e-6, "{model:?}");
        assert_eq!(fit_logistic::<Vec<f64>>(&[], &[], &FitOptions::new(1.0, &prior)), prior);
    }
}
