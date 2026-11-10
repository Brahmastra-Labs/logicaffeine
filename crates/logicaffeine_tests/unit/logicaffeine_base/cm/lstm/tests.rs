use super::*;

/// SplitMix64.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// The network's probability of `symbol` as the next byte: the product of its eight bits' probabilities.
fn probability(net: &Lstm, symbol: u8) -> f32 {
    (0..8)
        .rev()
        .fold((1.0f32, 1usize), |(p, node), i| {
            let bit = (symbol >> i) & 1 == 1;
            let p1 = net.front.unit(&net.current, node);
            (p * if bit { p1 } else { 1.0 - p1 }, node << 1 | usize::from(bit))
        })
        .0
}

/// Feed `bytes` to `net`, returning what coding them with its own next-byte distribution costs, in bits.
fn cost(net: &mut Lstm, bytes: &[u8]) -> f64 {
    let mut bits = 0.0;
    for &b in bytes {
        bits -= f64::from(probability(net, b)).log2();
        net.update(b);
    }
    bits
}

#[test]
fn the_exponential_is_accurate_monotone_and_saturates() {
    let mut last = 0.0f32;
    let mut x = -87.0f32;
    while x <= 88.0 {
        let e = exp(x);
        let want = f64::from(x).exp();
        assert!(((f64::from(e) - want) / want).abs() < 4e-7, "exp({x}) = {e}, want {want}");
        assert!(e >= last, "exp is monotone at {x}");
        last = e;
        x += 0.0137;
    }
    assert_eq!(exp(0.0), 1.0);
    assert_eq!(exp(-200.0), 0.0, "far below the range it is zero");
    assert!(exp(200.0).is_finite() && exp(200.0) >= exp(88.0), "far above, the largest it gives");
    for x in [-30.0f32, -3.0, -0.5, 0.0, 0.5, 3.0, 30.0] {
        assert!((sigmoid(x) - (1.0 / (1.0 + f64::from(-x).exp())) as f32).abs() < 1e-6);
        assert!((tanh(x) - f64::from(x).tanh() as f32).abs() < 2e-6);
    }
}

#[test]
fn a_dot_product_is_the_same_however_long_and_matches_a_plain_sum_closely() {
    let mut rng = Mix(1);
    for n in [0usize, 1, 7, 8, 9, 31, 64, 201] {
        let a: Vec<f32> = (0..n).map(|_| (rng.next() % 2001) as f32 / 1000.0 - 1.0).collect();
        let b: Vec<f32> = (0..n).map(|_| (rng.next() % 2001) as f32 / 1000.0 - 1.0).collect();
        let plain: f64 = a.iter().zip(&b).map(|(x, y)| f64::from(*x) * f64::from(*y)).sum();
        assert!((f64::from(dot(&a, &b)) - plain).abs() < 1e-4, "n {n}");
        assert_eq!(dot(&a, &b).to_bits(), dot(&a, &b).to_bits());
    }
}

#[test]
fn the_distribution_sums_to_one_and_the_bits_multiply_to_the_byte() {
    let mut net = Lstm::new(16, 2, 8);
    let text = b"probabilities of every byte, and of every bit of it, agree exactly with each other. ".repeat(4);
    for &b in &text {
        let total: f64 = (0..=255u8).map(|s| f64::from(probability(&net, s))).sum();
        assert!((total - 1.0).abs() < 1e-4, "sums to {total}");
        // Walking the byte's bits from the most significant, the bit probabilities multiply to the byte's.
        let mut product = 1.0f64;
        let mut c0 = 1u32;
        for i in (0..8).rev() {
            let bit = (b >> i) & 1 == 1;
            let p1 = f64::from(net.bit(c0));
            assert!((0.0..=1.0).contains(&p1));
            product *= if bit { p1 } else { 1.0 - p1 };
            c0 = c0 << 1 | u32::from(bit);
        }
        assert!((product - f64::from(probability(&net, b))).abs() < 1e-4 * product.max(1e-3), "{product} against {}", probability(&net, b));
        net.update(b);
    }
}

#[test]
fn two_networks_fed_the_same_bytes_agree_to_the_bit_and_a_clone_continues_in_step() {
    let mut rng = Mix(2);
    let data: Vec<u8> = (0..3000).map(|i| if i % 3 == 0 { (rng.next() % 26) as u8 + b'a' } else { b"the "[i % 4] }).collect();
    let (mut a, mut b) = (Lstm::new(24, 2, 10), Lstm::new(24, 2, 10));
    for (i, &x) in data.iter().enumerate() {
        for s in [0u8, x, 255] {
            assert_eq!(probability(&a, s).to_bits(), probability(&b, s).to_bits(), "byte {i}, symbol {s}");
        }
        a.update(x);
        b.update(x);
        if i == 1500 {
            b = a.fork();
        }
    }
}

#[test]
fn the_gradient_is_the_losss_slope() {
    // A tiny network: the analytic gradient of a window's loss against a finite difference, weight by weight.
    let mut net = Lstm::new(3, 2, 4);
    let warm = b"abcabcabd";
    for &b in warm {
        net.update(b);
    }
    let window = b"cabd";
    let (analytic, params) = net.window_gradient(window);
    let mut checked = 0;
    for k in 0..params {
        let h = 1e-2f32;
        let a = f64::from(analytic[k]);
        if a.abs() < 1e-3 && k % 7 != 0 {
            continue;
        }
        let numeric = (net.window_loss_with(window, k, h) - net.window_loss_with(window, k, -h)) / (2.0 * f64::from(h));
        if a.abs() < 1e-3 {
            assert!(numeric.abs() < 2e-3, "parameter {k}: no analytic gradient, but the loss moves by {numeric}");
            continue;
        }
        assert!((numeric - a).abs() <= 0.05 * numeric.abs().max(a.abs()) + 2e-4, "parameter {k}: analytic {a}, numeric {numeric}");
        checked += 1;
    }
    assert!(checked >= 100, "only {checked} parameters had a gradient to check");
}

#[test]
fn a_repeating_phrase_is_learnt_given_time() {
    let phrase = b"a long short-term memory learns the phrase it keeps seeing. ";
    let mut net = Lstm::new(32, 1, 10);
    let first = cost(&mut net, &phrase.repeat(60)) / (60 * phrase.len()) as f64;
    cost(&mut net, &phrase.repeat(180));
    let last = cost(&mut net, &phrase.repeat(60)) / (60 * phrase.len()) as f64;
    assert!(last < 1.0, "{last:.3} bits a byte after 240 repetitions");
    assert!(last < first / 3.0, "{last:.3} late against {first:.3} early");
}

/// Twenty kilobytes of words drawn from a vocabulary of a hundred, the dozen commonest more often.
fn word_text() -> Vec<u8> {
    let words: Vec<&str> = "the of and to in a is that for it as was with be by on not he this are or his from at which but \
        have an they you were her she there one all we their been has would when who will more no if out so said what up \
        its about into than them can only other new some could time these two may then do first any my now such like \
        our over man me even most made after also did many before must through back years where much your way well down \
        should because each just those people"
        .split_whitespace()
        .collect();
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut out: Vec<u8> = Vec::new();
    while out.len() < 20_000 {
        let w = if next() % 4 != 0 { words[(next() % words.len() as u64) as usize] } else { words[(next() % 12) as usize] };
        if !out.is_empty() {
            out.push(b' ');
        }
        out.extend_from_slice(w.as_bytes());
    }
    out
}

#[test]
fn on_text_it_beats_the_byte_frequencies_and_keeps_improving() {
    let text = word_text();
    let mut counts = [0u32; 256];
    let order0: Vec<f64> = text
        .iter()
        .enumerate()
        .map(|(i, &b)| {
            let p = (f64::from(counts[usize::from(b)]) + 0.5) / (i as f64 + 128.0);
            counts[usize::from(b)] += 1;
            -p.log2()
        })
        .collect();
    let mut net = Lstm::new(32, 1, 10);
    let quarter = text.len() / 4;
    let costs: Vec<f64> = text.chunks(quarter).map(|part| cost(&mut net, part)).collect();
    let lstm: f64 = costs.iter().sum();
    let zero: f64 = order0.iter().sum();
    assert!(lstm < 0.9 * zero, "LSTM {lstm:.0} bits against order-0 {zero:.0}");
    let last_zero: f64 = order0[3 * quarter..].iter().sum();
    assert!(costs[3] < 0.75 * last_zero, "the last quarter: {:.0} bits against order-0's {last_zero:.0}", costs[3]);
    assert!(costs[3] < costs[0] * 0.8, "it keeps improving: {:.0} bits in the last quarter, {:.0} in the first", costs[3], costs[0]);
}

#[test]
fn the_mixer_input_is_the_stretched_12_bit_probability_and_rises_with_it() {
    let mut net = Lstm::new(16, 1, 8);
    for &b in b"stretch the bit probability into the mixer's domain. ".repeat(20).iter() {
        let mut seen: Vec<(f32, i32)> = Vec::new();
        for c0 in 1..256u32 {
            let s = net.input(c0);
            let p = net.bit(c0);
            assert_eq!(s, stretch((p * 4096.0).round().clamp(1.0, 4095.0) as u16), "c0 {c0}, p {p}");
            assert!((-2047..=2047).contains(&s));
            seen.push((p, s));
        }
        seen.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert!(seen.windows(2).all(|w| w[0].1 <= w[1].1), "the input never falls as the probability rises");
        net.update(b);
    }
}

#[test]
fn training_on_the_worker_and_in_place_agree_to_the_bit() {
    let mut rng = Mix(5);
    let data: Vec<u8> = (0..4000).map(|i| if i % 5 == 0 { (rng.next() % 40) as u8 + b'0' } else { b"wire "[i % 5] }).collect();
    let mut threaded = Lstm::new(16, 2, 7);
    let mut here = Lstm::new(16, 2, 7).in_place();
    assert!(threaded.worker.is_some() || cfg!(target_family = "wasm"), "a worker where threads run");
    assert!(here.worker.is_none());
    for (i, &x) in data.iter().enumerate() {
        for c0 in [1u32, 2, 3, 77, 200] {
            assert_eq!(threaded.bit(c0).to_bits(), here.bit(c0).to_bits(), "byte {i}, partial byte {c0}");
        }
        threaded.update(x);
        here.update(x);
    }
}

impl Lstm {
    /// The same network training in place, with no worker.
    fn in_place(mut self) -> Self {
        self.settle();
        self.worker = None;
        self
    }

    /// A network that continues exactly as this one does, with a worker of its own.
    fn fork(&mut self) -> Self {
        self.settle();
        let pending = match &self.pending {
            Pending::Ready(done) => Pending::Ready(Done { trainer: done.trainer.clone(), trained: Arc::clone(&done.trained), steps: done.steps.clone() }),
            _ => Pending::None,
        };
        Self {
            horizon: self.horizon,
            front: Arc::clone(&self.front),
            window: self.window.clone(),
            spare: self.spare.clone(),
            filled: self.filled,
            current: self.current.clone(),
            trainer: self.trainer.clone(),
            pending,
            worker: Worker::spawn(),
        }
    }

    /// The window's steps from the state the current prediction started from: it read `current.symbol`, and then
    /// each byte of `window` but the last, each step's target the window's next byte.
    fn replay(&self, window: &[u8]) -> Vec<Step> {
        let n = self.front.cells;
        let mut prev = Step::new(&self.front);
        for l in 0..self.front.layers.len() {
            prev.h[l].copy_from_slice(&self.current.input[l][..n]);
            prev.c[l].copy_from_slice(&self.current.c_prev[l]);
        }
        let mut symbol = self.current.symbol;
        let mut steps = Vec::new();
        for &b in window {
            let mut step = Step::new(&self.front);
            self.front.forward(symbol, &prev.h, &prev.c, &mut step);
            step.target = b;
            symbol = b;
            prev = step.clone();
            steps.push(step);
        }
        steps
    }

    /// Every parameter in one order: the layers' symbol rows, the layers' matrices, the bit units.
    fn parameter(&mut self, k: usize) -> &mut f32 {
        let p = Arc::make_mut(&mut self.front);
        let mut k = k;
        for l in 0..p.layers.len() {
            if k < p.layers[l].symbol.len() {
                return &mut p.layers[l].symbol[k];
            }
            k -= p.layers[l].symbol.len();
        }
        for l in 0..p.layers.len() {
            if k < p.layers[l].weights.len() {
                return &mut p.layers[l].weights[k];
            }
            k -= p.layers[l].weights.len();
        }
        &mut p.output[k]
    }

    /// The gradient of `window`'s loss (replayed from the current prediction's start) in [`Self::parameter`] order,
    /// and how many parameters there are.
    fn window_gradient(&self, window: &[u8]) -> (Vec<f32>, usize) {
        let mut grad = Gradient::new(&self.front);
        grad.backward(&self.front, &self.replay(window));
        let flat: Vec<f32> = grad.symbol.concat().into_iter().chain(grad.weights.concat()).chain(grad.output).collect();
        let n = flat.len();
        (flat, n)
    }

    /// `window`'s loss in nats with parameter `k` moved by `delta`.
    fn window_loss_with(&mut self, window: &[u8], k: usize, delta: f32) -> f64 {
        let mut net = self.fork();
        *net.parameter(k) += delta;
        let mut loss = 0.0f64;
        for step in net.replay(window) {
            let mut node = 1usize;
            for i in (0..8).rev() {
                let bit = (step.target >> i) & 1 == 1;
                let x = f64::from(net.front.logit(&step, node));
                let p1 = 1.0 / (1.0 + (-x).exp());
                loss -= if bit { p1 } else { 1.0 - p1 }.ln();
                node = node << 1 | usize::from(bit);
            }
        }
        loss
    }
}

#[test]
fn the_adam_rate_falls_with_the_steps_to_its_floor() {
    assert_eq!(learning_rate(0), LEARNING_RATE);
    let at_half_life = learning_rate(RATE_STEPS as u32);
    assert!((at_half_life * 2f32.sqrt() - LEARNING_RATE).abs() < 1e-7, "{at_half_life} after {RATE_STEPS} steps");
    let mut last = LEARNING_RATE;
    for t in (0..4_000_000u32).step_by(10_007) {
        let rate = learning_rate(t);
        assert!(rate <= last && rate >= RATE_FLOOR, "step {t}: {rate} after {last}");
        last = rate;
    }
    assert_eq!(learning_rate(u32::MAX), RATE_FLOOR, "a long file trains at the floor");
}

#[test]
fn each_trained_window_is_one_step_down_the_rate() {
    let steps = |n: usize| {
        let mut net = Lstm::new(16, 2, 7).in_place();
        for i in 0..n {
            net.update(b"abcdefg"[i % 7]);
        }
        net.settle();
        // Between windows the trainer is home; once a window has trained, it waits in the window's result.
        match (&net.trainer, &net.pending) {
            (Some(trainer), _) => trainer.t,
            (None, Pending::Ready(done)) => done.trainer.t,
            _ => panic!("the trainer is home or in the last window's result"),
        }
    };
    let (once, twice) = (steps(140), steps(280));
    assert!(once > 0 && twice + 1 >= 2 * once, "{once} steps after 140 bytes, {twice} after 280");
}
