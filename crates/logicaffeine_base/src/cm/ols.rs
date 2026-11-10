//! An adaptive linear predictor of a 16-bit image's samples from their causal neighbours: least squares over the
//! samples so far, older ones forgotten geometrically, the weights re-solved every few samples. Arithmetic is `f64`
//! in one fixed order — additions, multiplications and divisions only, no fused operations — so every target computes
//! the same predictions and an encoder and its decoder agree bit for bit.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/ols/tests.rs"]
mod tests;

/// The neighbours a sample is predicted from, as (rows up, columns left): W, N, NW, NE, WW, NN, NNE, NWW, NEE, NNW.
const NEIGHBOURS: [(usize, isize); 10] = [(0, 1), (1, 0), (1, 1), (1, -1), (0, 2), (2, 0), (2, -1), (1, 2), (1, -2), (2, 1)];

/// The neighbours and a constant.
const N: usize = NEIGHBOURS.len() + 1;

/// The weight each update keeps of the statistics before it.
const FORGET: f64 = 0.998;

/// Samples between two solves of the weights.
const SOLVE_EVERY: u32 = 8;

/// A small ridge on the diagonal, so a flat region never makes the system singular.
const RIDGE: f64 = 1.0;

#[derive(Clone)]
pub struct Ols {
    /// The forgotten-weighted Gram matrix of the neighbours (its lower triangle) and their products with the samples.
    gram: [[f64; N]; N],
    moment: [f64; N],
    weights: [f64; N],
    since: u32,
}

impl Default for Ols {
    fn default() -> Self {
        Self::new()
    }
}

impl Ols {
    /// A predictor that starts as the mean of the sample to the left and the one above.
    pub fn new() -> Self {
        let mut weights = [0.0; N];
        weights[0] = 0.5;
        weights[1] = 0.5;
        Self { gram: [[0.0; N]; N], moment: [0.0; N], weights, since: 0 }
    }

    /// Sample `k`'s neighbours in an image `width` samples wide whose samples so far are `samples`, and a constant
    /// one; a neighbour outside the image reads as 0.
    pub fn features(samples: &[u16], k: usize, width: usize) -> [f64; N] {
        let (row, col) = (k / width, k % width);
        let mut f = [0.0; N];
        for (i, &(up, left)) in NEIGHBOURS.iter().enumerate() {
            let c = col as isize - left;
            if up <= row && c >= 0 && (c as usize) < width {
                let at = (row - up) * width + c as usize;
                if at < k {
                    f[i] = f64::from(samples[at]);
                }
            }
        }
        f[N - 1] = 1.0;
        f
    }

    /// The prediction for the sample with neighbours `f`, rounded and held to 16 bits.
    #[inline]
    pub fn predict(&self, f: &[f64; N]) -> u16 {
        let mut p = 0.0;
        for i in 0..N {
            p += self.weights[i] * f[i];
        }
        p.round().clamp(0.0, 65535.0) as u16
    }

    /// Learn that the sample with neighbours `f` was `sample`.
    pub fn update(&mut self, f: &[f64; N], sample: u16) {
        let x = f64::from(sample);
        for i in 0..N {
            for j in 0..=i {
                self.gram[i][j] = FORGET * self.gram[i][j] + f[i] * f[j];
            }
            self.moment[i] = FORGET * self.moment[i] + f[i] * x;
        }
        self.since += 1;
        if self.since >= SOLVE_EVERY {
            self.since = 0;
            self.solve();
        }
    }

    /// Solve `(G + ridge·I) w = m` by an LDLᵀ factorization; a non-positive pivot leaves the weights as they were.
    fn solve(&mut self) {
        let mut l = [[0.0; N]; N];
        let mut d = [0.0; N];
        for i in 0..N {
            for j in 0..=i {
                let mut s = self.gram[i][j] + if i == j { RIDGE } else { 0.0 };
                for k in 0..j {
                    s -= l[i][k] * l[j][k] * d[k];
                }
                if i == j {
                    if !(s > 0.0) {
                        return;
                    }
                    d[i] = s;
                    l[i][i] = 1.0;
                } else {
                    l[i][j] = s / d[j];
                }
            }
        }
        let mut y = [0.0; N];
        for i in 0..N {
            let mut s = self.moment[i];
            for k in 0..i {
                s -= l[i][k] * y[k];
            }
            y[i] = s;
        }
        let mut w = [0.0; N];
        for i in (0..N).rev() {
            let mut s = y[i] / d[i];
            for k in i + 1..N {
                s -= l[k][i] * w[k];
            }
            w[i] = s;
        }
        if w.iter().all(|v| v.is_finite()) {
            self.weights = w;
        }
    }
}
