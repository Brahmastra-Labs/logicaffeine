//! A long short-term memory network over the byte stream: it reads each byte and predicts the next one's
//! distribution, from which every bit's probability follows, a mixer input unlike any hashed context's — the network
//! carries what it has learnt about the text in its state rather than looking it up.
//!
//! Each layer's cells have a coupled input and forget gate (what a cell takes in, it forgets of its old content), a
//! candidate and an output gate; a layer sees the last byte (through a row of weights per byte value), its own last
//! output and the layer below's. The next byte is predicted a bit at a time, as the coder codes it: each of the 255
//! nodes of the byte's bit tree (a partial byte) has a logistic unit over every layer's output, so a byte's
//! probability is the product of the eight its bits pass through. The network learns online: every `horizon` bytes it
//! backpropagates the window's loss through time and takes an Adam step. The training runs on a worker thread while
//! the next window's bytes are read (or in place, where there are no threads), and its result is taken at the end of
//! that window either way: a window's bytes are read with the weights trained up to the window before last, so the
//! network is the same wherever it trains.
//!
//! All arithmetic is `f32` in a fixed order — every sum over a vector runs in eight fixed lanes, the exponential is
//! built from additions and multiplications — so the predictions are the same bits on every machine. The window's
//! steps, the gradient and the backpropagation's temporaries are allocated once and reused.

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_base/cm/lstm/tests.rs"]
mod tests;

use crate::entropy::stretch;
use std::sync::Arc;

const SYMBOLS: usize = 256;

/// Adam's moment decays and its denominator's floor; moments below `FLUSH` become zero, so a gradient that has died
/// away never leaves denormal numbers behind.
const BETA1: f32 = 0.9;
const BETA2: f32 = 0.999;
const EPSILON: f32 = 1e-6;
const FLUSH: f32 = 1e-30;

/// The nodes of a byte's bit tree: partial bytes `1..=255` (1 then the bits so far).
const NODES: usize = 255;

/// How sure a bit's unit may be: its probability stays in `[P_MIN, 1 − P_MIN]`.
const P_MIN: f32 = 1.0 / 65536.0;

/// The learning rate of an Adam step.
const LEARNING_RATE: f32 = 0.005;

/// The input gates' initial bias: a fresh cell keeps most of its content.
const INPUT_BIAS: f32 = -1.0;

/// `ln 2` split so that `n · LN2_HI` is exact for every exponent `n` the exponential meets.
const LN2_HI: f32 = 0.693_359_4;
const LN2_LO: f32 = -2.121_944_4e-4;

/// `e^x` to a few units in the last place, from additions and multiplications only: `x = n ln 2 + r` with
/// `|r| ≤ ln 2 / 2`, `e^r` by its degree-7 Taylor polynomial, scaled by `2^n` exactly. Below the normal range it is
/// zero; above `88` it is `e^88`.
pub fn exp(x: f32) -> f32 {
    if x < -87.0 {
        return 0.0;
    }
    let x = x.min(88.0);
    let n = (x * std::f32::consts::LOG2_E + 0.5).floor();
    let r = (x - n * LN2_HI) - n * LN2_LO;
    let p = 1.0 + r * (1.0 + r * (0.5 + r * (1.0 / 6.0 + r * (1.0 / 24.0 + r * (1.0 / 120.0 + r * (1.0 / 720.0 + r * (1.0 / 5040.0)))))));
    p * f32::from_bits(((n as i32 + 127) as u32) << 23)
}

pub fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + exp(-x))
}

pub fn tanh(x: f32) -> f32 {
    1.0 - 2.0 / (1.0 + exp(2.0 * x))
}

/// `a · b` summed in eight fixed lanes, then the lanes pairwise, then the tail: one order however it is compiled.
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len());
    let mut lanes = [0.0f32; 8];
    let whole = a.len() / 8 * 8;
    for (x, y) in a[..whole].chunks_exact(8).zip(b[..whole].chunks_exact(8)) {
        for j in 0..8 {
            lanes[j] += x[j] * y[j];
        }
    }
    let mut tail = 0.0f32;
    for (x, y) in a[whole..].iter().zip(&b[whole..]) {
        tail += x * y;
    }
    ((lanes[0] + lanes[4]) + (lanes[1] + lanes[5])) + ((lanes[2] + lanes[6]) + (lanes[3] + lanes[7])) + tail
}

/// `y += a · x`.
fn axpy(y: &mut [f32], a: f32, x: &[f32]) {
    for (y, x) in y.iter_mut().zip(x) {
        *y += a * x;
    }
}

/// One layer's parameters: per byte value a row of its three gates' pre-activations (input, candidate, output; `3H`),
/// and the matrix from its inputs — its own last output, the layer below's output (above the first layer), and a
/// bias — to the same `3H`.
#[derive(Clone)]
struct Layer {
    symbol: Vec<f32>,
    weights: Vec<f32>,
    inputs: usize,
}

/// The network's parameters: its layers and the bit units' weights (per node of the bit tree a row over every
/// layer's output and a bias).
#[derive(Clone)]
struct Params {
    cells: usize,
    layers: Vec<Layer>,
    output: Vec<f32>,
}

/// What one step of the network computed, kept for backpropagation: the byte it read, per layer its input vector,
/// gate activations, cell content before and after (and after its `tanh`) and output, every layer's output with a
/// bias (what the bit units read), and the byte that came.
#[derive(Clone)]
struct Step {
    symbol: u8,
    input: Vec<Vec<f32>>,
    gates: Vec<Vec<f32>>,
    c_prev: Vec<Vec<f32>>,
    c: Vec<Vec<f32>>,
    tanh_c: Vec<Vec<f32>>,
    h: Vec<Vec<f32>>,
    features: Vec<f32>,
    target: u8,
}

impl Step {
    /// A step's buffers for `params`, zero: the state before the first byte.
    fn new(params: &Params) -> Self {
        let n = params.cells;
        let per = |len: usize| vec![vec![0.0f32; len]; params.layers.len()];
        Self {
            symbol: 0,
            input: params.layers.iter().map(|l| vec![0.0; l.inputs]).collect(),
            gates: per(3 * n),
            c_prev: per(n),
            c: per(n),
            tanh_c: per(n),
            h: per(n),
            features: vec![0.0; params.layers.len() * n + 1],
            target: 0,
        }
    }
}

impl Params {
    /// One step into `out`: read `symbol` after a step whose outputs and cell contents are `prev_h` and `prev_c`, and
    /// predict the next byte.
    fn forward(&self, symbol: u8, prev_h: &[Vec<f32>], prev_c: &[Vec<f32>], out: &mut Step) {
        let n = self.cells;
        let h3 = 3 * n;
        out.symbol = symbol;
        for (l, layer) in self.layers.iter().enumerate() {
            out.c_prev[l].copy_from_slice(&prev_c[l]);
            let (below, rest) = out.h.split_at_mut(l);
            let input = &mut out.input[l];
            input[..n].copy_from_slice(&prev_h[l]);
            if l > 0 {
                input[n..2 * n].copy_from_slice(&below[l - 1]);
            }
            input[layer.inputs - 1] = 1.0;
            let row = &layer.symbol[usize::from(symbol) * h3..(usize::from(symbol) + 1) * h3];
            let gates = &mut out.gates[l];
            for r in 0..h3 {
                let x = row[r] + dot(&layer.weights[r * layer.inputs..(r + 1) * layer.inputs], input);
                gates[r] = if (n..2 * n).contains(&r) { tanh(x) } else { sigmoid(x) };
            }
            let (c, tanh_c, h) = (&mut out.c[l], &mut out.tanh_c[l], &mut rest[0]);
            for j in 0..n {
                c[j] = (1.0 - gates[j]) * prev_c[l][j] + gates[j] * gates[n + j];
                tanh_c[j] = tanh(c[j]);
                h[j] = gates[2 * n + j] * tanh_c[j];
            }
        }
        for (l, h) in out.h.iter().enumerate() {
            out.features[l * n..(l + 1) * n].copy_from_slice(h);
        }
        let last = out.features.len() - 1;
        out.features[last] = 1.0;
    }

    /// Node `node`'s logit for `step`'s next bit.
    fn logit(&self, step: &Step, node: usize) -> f32 {
        let width = step.features.len();
        dot(&self.output[(node - 1) * width..node * width], &step.features)
    }

    /// Node `node`'s probability that `step`'s next bit is 1.
    fn unit(&self, step: &Step, node: usize) -> f32 {
        sigmoid(self.logit(step, node)).clamp(P_MIN, 1.0 - P_MIN)
    }
}

/// Adam's two moments for a parameter vector.
#[derive(Clone)]
struct Moments {
    m: Vec<f32>,
    v: Vec<f32>,
}

impl Moments {
    fn new(n: usize) -> Self {
        Self { m: vec![0.0; n], v: vec![0.0; n] }
    }

    /// One Adam step of `params[range]` along `grad[range]`, with the bias corrections `c1 = 1 − β1^t`, `c2 = 1 − β2^t`.
    fn step(&mut self, params: &mut [f32], grad: &[f32], range: std::ops::Range<usize>, lr: f32, c1: f32, c2: f32) {
        for i in range {
            let g = grad[i];
            let m = BETA1 * self.m[i] + (1.0 - BETA1) * g;
            let v = BETA2 * self.v[i] + (1.0 - BETA2) * g * g;
            self.m[i] = if m.abs() < FLUSH { 0.0 } else { m };
            self.v[i] = if v < FLUSH { 0.0 } else { v };
            params[i] -= lr * (self.m[i] / c1) / ((self.v[i] / c2).sqrt() + EPSILON);
        }
    }
}

/// The gradient of a window's loss, and the backpropagation's temporaries.
#[derive(Clone)]
struct Gradient {
    symbol: Vec<Vec<f32>>,
    weights: Vec<Vec<f32>>,
    output: Vec<f32>,
    /// Per layer, the byte values whose symbol rows the window touched, in first-touch order.
    touched: Vec<Vec<u8>>,
    dh_next: Vec<Vec<f32>>,
    dc_next: Vec<Vec<f32>>,
    dfeatures: Vec<f32>,
    from_above: Vec<f32>,
    dpre: Vec<f32>,
    dinput: Vec<f32>,
}

impl Gradient {
    fn new(params: &Params) -> Self {
        let n = params.cells;
        let depth = params.layers.len();
        Self {
            symbol: params.layers.iter().map(|l| vec![0.0; l.symbol.len()]).collect(),
            weights: params.layers.iter().map(|l| vec![0.0; l.weights.len()]).collect(),
            output: vec![0.0; params.output.len()],
            touched: vec![Vec::new(); depth],
            dh_next: vec![vec![0.0; n]; depth],
            dc_next: vec![vec![0.0; n]; depth],
            dfeatures: vec![0.0; depth * n + 1],
            from_above: vec![0.0; n],
            dpre: vec![0.0; 3 * n],
            dinput: vec![0.0; params.layers.iter().map(|l| l.inputs).max().unwrap_or(0)],
        }
    }

    /// Back to zero: the dense parts whole, the symbol rows the last window touched.
    fn clear(&mut self, cells: usize) {
        let h3 = 3 * cells;
        for (rows, touched) in self.symbol.iter_mut().zip(&mut self.touched) {
            for &s in touched.iter() {
                rows[usize::from(s) * h3..(usize::from(s) + 1) * h3].fill(0.0);
            }
            touched.clear();
        }
        for w in &mut self.weights {
            w.fill(0.0);
        }
        self.output.fill(0.0);
    }

    /// Add the gradient of `steps`' loss (the sum over their bytes' bits of `−ln p(bit)`) by backpropagation through
    /// them.
    fn backward(&mut self, params: &Params, steps: &[Step]) {
        let n = params.cells;
        let h3 = 3 * n;
        let depth = params.layers.len();
        let width = depth * n + 1;
        for v in self.dh_next.iter_mut().chain(self.dc_next.iter_mut()) {
            v.fill(0.0);
        }
        for step in steps.iter().rev() {
            self.dfeatures.fill(0.0);
            let mut node = 1usize;
            for i in (0..8).rev() {
                let bit = (step.target >> i) & 1 == 1;
                let p1 = sigmoid(params.logit(step, node));
                let dz = p1 - if bit { 1.0 } else { 0.0 };
                axpy(&mut self.output[(node - 1) * width..node * width], dz, &step.features);
                axpy(&mut self.dfeatures, dz, &params.output[(node - 1) * width..node * width]);
                node = node << 1 | usize::from(bit);
            }
            self.from_above.fill(0.0);
            for l in (0..depth).rev() {
                let layer = &params.layers[l];
                let (gates, tanh_c, c_prev) = (&step.gates[l], &step.tanh_c[l], &step.c_prev[l]);
                for j in 0..n {
                    let dh = self.dfeatures[l * n + j] + self.dh_next[l][j] + if l + 1 < depth { self.from_above[j] } else { 0.0 };
                    let (i, g, o) = (gates[j], gates[n + j], gates[2 * n + j]);
                    let dc = self.dc_next[l][j] + dh * o * (1.0 - tanh_c[j] * tanh_c[j]);
                    self.dpre[j] = dc * (g - c_prev[j]) * i * (1.0 - i);
                    self.dpre[n + j] = dc * i * (1.0 - g * g);
                    self.dpre[2 * n + j] = dh * tanh_c[j] * o * (1.0 - o);
                    self.dc_next[l][j] = dc * (1.0 - i);
                }
                let row = usize::from(step.symbol);
                if !self.touched[l].contains(&step.symbol) {
                    self.touched[l].push(step.symbol);
                }
                for (g, d) in self.symbol[l][row * h3..(row + 1) * h3].iter_mut().zip(&self.dpre) {
                    *g += d;
                }
                let dinput = &mut self.dinput[..layer.inputs];
                dinput.fill(0.0);
                for (r, &d) in self.dpre.iter().enumerate() {
                    axpy(&mut self.weights[l][r * layer.inputs..(r + 1) * layer.inputs], d, &step.input[l]);
                    axpy(dinput, d, &layer.weights[r * layer.inputs..(r + 1) * layer.inputs]);
                }
                self.dh_next[l].copy_from_slice(&dinput[..n]);
                if l > 0 {
                    self.from_above.copy_from_slice(&dinput[n..2 * n]);
                }
            }
        }
    }
}

/// What trains the network: the master weights, Adam's moments, and the gradient's buffers.
#[derive(Clone)]
struct Trainer {
    rate: f32,
    master: Params,
    symbol_moments: Vec<Moments>,
    weight_moments: Vec<Moments>,
    output_moments: Moments,
    /// `β1^t` and `β2^t` after `t` Adam steps.
    beta1_t: f32,
    beta2_t: f32,
    grad: Gradient,
}

impl Trainer {
    /// One Adam step of the master weights along the gradient of `steps`, which ran on `ran_on`; a symbol row the
    /// window never read keeps its weights and moments.
    fn train(&mut self, ran_on: &Params, steps: &[Step]) {
        self.grad.backward(ran_on, steps);
        self.beta1_t *= BETA1;
        self.beta2_t *= BETA2;
        let (c1, c2) = (1.0 - self.beta1_t, 1.0 - self.beta2_t);
        let h3 = 3 * self.master.cells;
        let grad = &self.grad;
        for l in 0..self.master.layers.len() {
            for &s in &grad.touched[l] {
                let row = usize::from(s) * h3..(usize::from(s) + 1) * h3;
                self.symbol_moments[l].step(&mut self.master.layers[l].symbol, &grad.symbol[l], row, self.rate, c1, c2);
            }
            let all = 0..self.master.layers[l].weights.len();
            self.weight_moments[l].step(&mut self.master.layers[l].weights, &grad.weights[l], all, self.rate, c1, c2);
        }
        let all = 0..self.master.output.len();
        self.output_moments.step(&mut self.master.output, &grad.output, all, self.rate, c1, c2);
        self.grad.clear(self.master.cells);
    }
}

/// A window to train on: the trainer, the weights the window's steps ran on, and the steps.
struct Job {
    trainer: Trainer,
    ran_on: Arc<Params>,
    steps: Vec<Step>,
}

/// A trained window: the trainer back, the weights it trained to, and the steps' buffers for reuse.
struct Done {
    trainer: Trainer,
    trained: Arc<Params>,
    steps: Vec<Step>,
}

impl Job {
    fn run(mut self) -> Done {
        self.trainer.train(&self.ran_on, &self.steps);
        let trained = Arc::new(self.trainer.master.clone());
        Done { trainer: self.trainer, trained, steps: self.steps }
    }
}

/// A thread that runs jobs as they come, in order.
struct Worker {
    jobs: Option<std::sync::mpsc::Sender<Job>>,
    done: std::sync::mpsc::Receiver<Done>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Worker {
    /// A worker, where the platform runs threads.
    fn spawn() -> Option<Self> {
        if cfg!(target_family = "wasm") {
            return None;
        }
        let (jobs, inbox) = std::sync::mpsc::channel::<Job>();
        let (outbox, done) = std::sync::mpsc::channel::<Done>();
        let thread = std::thread::Builder::new()
            .name("cm-lstm".into())
            .spawn(move || {
                for job in inbox {
                    if outbox.send(job.run()).is_err() {
                        break;
                    }
                }
            })
            .ok()?;
        Some(Self { jobs: Some(jobs), done, thread: Some(thread) })
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The last window's training: none yet, done (in place), or running on the worker.
enum Pending {
    None,
    Ready(Done),
    Running,
}

pub struct Lstm {
    horizon: usize,
    /// The weights the steps run on.
    front: Arc<Params>,
    /// The window's steps (the first `filled` of them done), the buffers for the next window, and the step whose
    /// prediction is for the next byte.
    window: Vec<Step>,
    spare: Vec<Step>,
    filled: usize,
    current: Step,
    /// The trainer, when no window holds it, and the last window's training.
    trainer: Option<Trainer>,
    pending: Pending,
    worker: Option<Worker>,
}

impl Lstm {
    /// A network of `layers` layers of `cells` cells, trained every `horizon` bytes, its weights drawn from a fixed
    /// sequence (so every network of a shape starts the same).
    pub fn new(cells: usize, layers: usize, horizon: usize) -> Self {
        assert!(cells > 0 && layers > 0 && horizon > 0);
        let mut seed = 0x4C53_544D_u64;
        let mut uniform = |scale: f32| {
            seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            ((z >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0) * scale
        };
        let layers: Vec<Layer> = (0..layers)
            .map(|l| {
                let inputs = cells + if l > 0 { cells } else { 0 } + 1;
                let scale = 1.0 / (inputs as f32).sqrt();
                let symbol = (0..SYMBOLS * 3 * cells).map(|_| uniform(0.1)).collect();
                let mut weights: Vec<f32> = (0..3 * cells * inputs).map(|_| uniform(scale)).collect();
                for r in 0..cells {
                    weights[r * inputs + inputs - 1] = INPUT_BIAS;
                }
                Layer { symbol, weights, inputs }
            })
            .collect();
        let width = layers.len() * cells + 1;
        let params = Params { cells, layers, output: vec![0.0; NODES * width] };
        let zero = Step::new(&params);
        let mut current = Step::new(&params);
        params.forward(0, &zero.h, &zero.c, &mut current);
        let trainer = Trainer {
            rate: LEARNING_RATE,
            symbol_moments: params.layers.iter().map(|l| Moments::new(l.symbol.len())).collect(),
            weight_moments: params.layers.iter().map(|l| Moments::new(l.weights.len())).collect(),
            output_moments: Moments::new(params.output.len()),
            beta1_t: 1.0,
            beta2_t: 1.0,
            grad: Gradient::new(&params),
            master: params.clone(),
        };
        Self {
            horizon,
            window: (0..horizon).map(|_| Step::new(&params)).collect(),
            spare: (0..horizon).map(|_| Step::new(&params)).collect(),
            filled: 0,
            current,
            front: Arc::new(params),
            trainer: Some(trainer),
            pending: Pending::None,
            worker: Worker::spawn(),
        }
    }

    /// The probability that the next bit is 1, given the byte's bits so far (`c0`: 1 then those bits, 1..=255).
    pub fn bit(&self, c0: u32) -> f32 {
        self.front.unit(&self.current, c0 as usize)
    }

    /// [`Self::bit`] for the mixer: stretched, from a 12-bit probability.
    pub fn input(&self, c0: u32) -> i32 {
        let p12 = (self.bit(c0) * 4096.0).round().clamp(1.0, 4095.0) as u16;
        stretch(p12)
    }

    /// Learn that the next byte was `byte` (handing the window over for training once it is `horizon` bytes long),
    /// then read it and predict the one after.
    pub fn update(&mut self, byte: u8) {
        self.current.target = byte;
        std::mem::swap(&mut self.window[self.filled], &mut self.current);
        let done = &self.window[self.filled];
        self.front.forward(byte, &done.h, &done.c, &mut self.current);
        self.filled += 1;
        if self.filled == self.horizon {
            self.hand_over();
            self.filled = 0;
        }
    }

    /// Wait for the last window's training, if it is running.
    fn settle(&mut self) {
        if matches!(self.pending, Pending::Running) {
            let worker = self.worker.as_ref().expect("a running window has a worker");
            self.pending = Pending::Ready(worker.done.recv().expect("the LSTM worker stopped"));
        }
    }

    /// The window is full: take the last window's training (its weights become the ones the next window runs on),
    /// and start this window's.
    fn hand_over(&mut self) {
        self.settle();
        let ran_on = Arc::clone(&self.front);
        if let Pending::Ready(done) = std::mem::replace(&mut self.pending, Pending::None) {
            self.trainer = Some(done.trainer);
            self.front = done.trained;
            self.spare = done.steps;
        }
        let steps = std::mem::replace(&mut self.window, std::mem::take(&mut self.spare));
        let job = Job { trainer: self.trainer.take().expect("the trainer is home"), ran_on, steps };
        self.pending = match &self.worker {
            Some(worker) => match worker.jobs.as_ref().expect("a worker takes jobs").send(job) {
                Ok(()) => Pending::Running,
                Err(failed) => Pending::Ready(failed.0.run()),
            },
            None => Pending::Ready(job.run()),
        };
    }
}
