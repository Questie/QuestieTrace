// L2-regularized logistic regression, fitted with Newton's method. Feature counts are small
// (a few per signal), so a dense solve per iteration is instant and needs no dependency.

export interface LogisticModel {
  intercept: number;
  weights: number[];
}

export interface FitOptions {
  /** Ridge strength on the weights. */
  ridge: number;
  /** Ridge on the intercept. Tiny by default: it only stops divergence when every label is equal. */
  interceptRidge?: number;
  /** Center of the ridge, which also fixes the width when there are no rows. Defaults to all zeros. */
  prior?: LogisticModel;
  maxIterations?: number;
}

function sigmoid(z: number): number {
  return z >= 0 ? 1 / (1 + Math.exp(-z)) : Math.exp(z) / (1 + Math.exp(z));
}

export function logit(p: number): number {
  return Math.log(p / (1 - p));
}

export function predict(model: LogisticModel, x: readonly number[]): number {
  let z = model.intercept;
  for (let j = 0; j < x.length; j++) z += model.weights[j] * x[j];
  return sigmoid(z);
}

export function fitLogistic(rows: readonly (readonly number[])[], labels: readonly number[], options: FitOptions): LogisticModel {
  const width = rows[0]?.length ?? options.prior?.weights.length ?? 0;
  const prior = options.prior ?? { intercept: 0, weights: new Array<number>(width).fill(0) };
  const interceptRidge = options.interceptRidge ?? 1e-3;
  // beta[0] is the intercept, beta[1..] the weights.
  const beta = [prior.intercept, ...prior.weights];
  const center = [prior.intercept, ...prior.weights];
  const ridge = [interceptRidge, ...new Array<number>(width).fill(options.ridge)];
  const size = width + 1;
  // Rows are sparse (a handful of signals claim any one edge), so only their non-zero columns are visited.
  const sparse = rows.map((row) => {
    const columns = [0];
    const values = [1];
    row.forEach((value, j) => {
      if (value !== 0) {
        columns.push(j + 1);
        values.push(value);
      }
    });
    return { columns, values };
  });

  for (let iteration = 0; iteration < (options.maxIterations ?? 50); iteration++) {
    const gradient = beta.map((value, j) => ridge[j] * (value - center[j]));
    const hessian = Array.from({ length: size }, (_, j) => Array.from({ length: size }, (_, k) => (j === k ? ridge[j] : 0)));
    sparse.forEach(({ columns, values }, i) => {
      let z = 0;
      for (let a = 0; a < columns.length; a++) z += beta[columns[a]] * values[a];
      const p = sigmoid(z);
      const curvature = p * (1 - p);
      for (let a = 0; a < columns.length; a++) {
        gradient[columns[a]] += (p - labels[i]) * values[a];
        for (let b = 0; b < columns.length; b++) hessian[columns[a]][columns[b]] += curvature * values[a] * values[b];
      }
    });
    const step = solve(hessian, gradient);
    let largest = 0;
    for (let j = 0; j < size; j++) {
      beta[j] -= step[j];
      largest = Math.max(largest, Math.abs(step[j]));
    }
    if (largest < 1e-7) break;
  }
  return { intercept: beta[0], weights: beta.slice(1) };
}

/** Solves A x = b for a symmetric positive definite A (Cholesky). */
function solve(a: number[][], b: number[]): number[] {
  const n = b.length;
  const l = Array.from({ length: n }, () => new Array<number>(n).fill(0));
  for (let i = 0; i < n; i++) {
    for (let j = 0; j <= i; j++) {
      let sum = a[i][j];
      for (let k = 0; k < j; k++) sum -= l[i][k] * l[j][k];
      l[i][j] = i === j ? Math.sqrt(Math.max(sum, 1e-12)) : sum / l[j][j];
    }
  }
  const y = new Array<number>(n).fill(0);
  for (let i = 0; i < n; i++) {
    let sum = b[i];
    for (let k = 0; k < i; k++) sum -= l[i][k] * y[k];
    y[i] = sum / l[i][i];
  }
  const x = new Array<number>(n).fill(0);
  for (let i = n - 1; i >= 0; i--) {
    let sum = y[i];
    for (let k = i + 1; k < n; k++) sum -= l[k][i] * x[k];
    x[i] = sum / l[i][i];
  }
  return x;
}
