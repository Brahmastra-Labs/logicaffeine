//! The **PMU banking model** — P2 of the Plasticine lane: the Pattern Memory Unit's scratchpad as the papers
//! describe it (ISCA'17 §3.2), with an executable meaning.
//!
//! A PMU is `banks` SRAM banks of `words_per_bank` 32-bit words (ISCA'17: 16 banks × 16 KB = 256 KB) behind an
//! address-calculation datapath of `stages` FU stages. Banking modes: **strided** (a configurable granularity
//! so a stride-`s` vector access spreads across all banks), **FIFO**, **line buffer**, and **duplication**
//! (every bank holds a copy, so any gather is conflict-free at the cost of broadcast writes). **N-buffering**
//! partitions each bank's address space into `n_buffers` disjoint regions so a metapipeline's producer writes
//! region `t+1` while its consumer reads region `t`.
//!
//! The executable meaning: [`Pmu`] is banked storage whose reads/writes equal a flat heap's (the banking is a
//! bijection), and every vector access costs `conflicts` cycles = the most lanes that land in one bank. That
//! cost is the stage's **memory II** (`metapipeline::Stage::memory_ii`). MODELED from the paper; never a
//! Plasticine simulation.

/// How addresses map to banks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub enum BankingMode {
    /// `bank = (addr / granularity) % banks`: consecutive `granularity`-word chunks rotate through the banks,
    /// so a vector access with stride == granularity hits every bank once.
    Strided { granularity: usize },
    /// A queue: consecutive pushes rotate through the banks (`granularity` 1); pops in push order.
    Fifo,
    /// Sliding window over rows of `width` words: `bank = (addr % width) % banks`, so a row's lanes spread.
    LineBuffer { width: usize },
    /// Every bank holds a full copy: any gather reads conflict-free; a write is broadcast to all banks.
    Duplication,
}

/// A PMU's published shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct PmuSpec {
    pub banks: usize,
    pub words_per_bank: usize,
    /// N-buffering factor (1 = single buffer, 2 = double buffer, …).
    pub n_buffers: usize,
    /// Address-datapath FU stages.
    pub stages: usize,
}

impl PmuSpec {
    /// ISCA'17 Table 3: 16 banks × 16 KB (4096 32-bit words), 4 address-datapath stages, single-buffered.
    pub fn isca17() -> PmuSpec {
        PmuSpec {
            banks: 16,
            words_per_bank: 4096,
            n_buffers: 1,
            stages: 4,
        }
    }
    /// Total words.
    pub fn capacity_words(&self) -> usize {
        self.banks * self.words_per_bank
    }
    /// Words available to ONE buffer under N-buffering (each bank's space is split `n_buffers` ways; when
    /// the split is uneven the smallest region is what a tile must fit).
    pub fn buffer_words(&self) -> usize {
        self.capacity_words() / self.n_buffers.max(1)
    }
    /// The disjoint, exhaustive `[first, last)` word ranges of the N buffers in flat word addresses; the
    /// remainder of an uneven split goes to the last region so the regions always cover the capacity.
    pub fn n_buffer_regions(&self) -> Vec<(usize, usize)> {
        let n = self.n_buffers.max(1);
        let per = self.capacity_words() / n;
        (0..n)
            .map(|i| {
                (
                    i * per,
                    if i + 1 == n {
                        self.capacity_words()
                    } else {
                        (i + 1) * per
                    },
                )
            })
            .collect()
    }
    /// Words a banking mode can address: everything for strided / FIFO / duplication; a line buffer holds
    /// whole rows, `ceil(width / banks)` words of every row in each bank, so its capacity is the whole rows
    /// that fit — the rest of the SRAM is unaddressable in that mode.
    pub fn usable_words(&self, mode: BankingMode) -> usize {
        match mode {
            BankingMode::LineBuffer { width } => {
                let w = width.max(1);
                let cols_per_bank = (w + self.banks - 1) / self.banks;
                (self.words_per_bank / cols_per_bank) * w
            }
            // Strided banking rotates `granularity`-word chunks through the banks: a bank holds whole chunks,
            // so the usable capacity is the whole chunk-rounds that fit (`granularity` 1 = everything).
            BankingMode::Strided { granularity } => {
                let g = granularity.max(1);
                (self.words_per_bank / g) * g * self.banks
            }
            // Duplication replicates every word into every bank: the usable capacity is ONE bank's worth.
            BankingMode::Duplication => self.words_per_bank,
            BankingMode::Fifo => self.capacity_words(),
        }
    }
    /// Does a tile of `words` fit one buffer? Typed refusal otherwise.
    pub fn tile_fits(&self, words: usize) -> Result<(), String> {
        if words <= self.buffer_words() {
            Ok(())
        } else {
            Err(format!("a {words}-word tile does not fit one of {} buffers of {} words ({} banks x {} words)", self.n_buffers.max(1), self.buffer_words(), self.banks, self.words_per_bank))
        }
    }
}

/// One vector access: lane `l` reads/writes `base + stride * l` for `l` in `0..lanes`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AffineAccess {
    pub base: usize,
    pub stride: usize,
    pub lanes: usize,
}

impl AffineAccess {
    pub fn addrs(&self) -> Vec<usize> {
        (0..self.lanes)
            .map(|l| self.base + self.stride * l)
            .collect()
    }
}

/// The bank an address lives in under `mode`.
pub fn bank_of(addr: usize, mode: BankingMode, banks: usize) -> usize {
    match mode {
        BankingMode::Strided { granularity } => (addr / granularity.max(1)) % banks,
        BankingMode::Fifo => addr % banks,
        BankingMode::LineBuffer { width } => (addr % width.max(1)) % banks,
        BankingMode::Duplication => 0,
    }
}

/// Cycles a vector access costs: the most lanes that land in one bank (1 = conflict-free). Under
/// duplication a read is always 1 (every bank has the data; lanes are served from distinct copies).
pub fn conflicts(access: &AffineAccess, mode: BankingMode, spec: &PmuSpec) -> usize {
    if access.lanes == 0 {
        return 0;
    }
    if mode == BankingMode::Duplication {
        return 1;
    }
    let mut per_bank = vec![0usize; spec.banks];
    for a in access.addrs() {
        per_bank[bank_of(a, mode, spec.banks)] += 1;
    }
    per_bank.into_iter().max().unwrap_or(1).max(1)
}

/// The strided-banking configuration that makes a stride-`stride` vector access conflict-free: granularity
/// = stride (lane `l` lands in bank `(base/stride + l) % banks`, all distinct while `lanes <= banks`).
pub fn strided_banking_for(stride: usize) -> BankingMode {
    BankingMode::Strided {
        granularity: stride.max(1),
    }
}

/// The memory II of a stage: the costliest of its vector accesses per iteration.
pub fn memory_ii(accesses: &[AffineAccess], mode: BankingMode, spec: &PmuSpec) -> usize {
    accesses
        .iter()
        .map(|a| conflicts(a, mode, spec))
        .max()
        .unwrap_or(1)
        .max(1)
}

/// FU stages the address datapath needs for an affine address `base + Σ stride_k * idx_k` over `dims`
/// counters (+1 for the lane offset when vectorized): one multiply-add per dimension, chained.
pub fn address_datapath_stages(dims: usize, vectorized: bool) -> usize {
    dims + if vectorized { 1 } else { 0 }
}

/// Gate an address pattern against the PMU's datapath depth — typed refusal.
pub fn address_fits(spec: &PmuSpec, dims: usize, vectorized: bool) -> Result<usize, String> {
    let need = address_datapath_stages(dims, vectorized);
    if need <= spec.stages {
        Ok(need)
    } else {
        Err(format!(
            "a {dims}-dimensional{} address needs {need} FU stages; the PMU datapath has {}",
            if vectorized { " vectorized" } else { "" },
            spec.stages
        ))
    }
}

/// Banked storage with a flat-address interface: the executable meaning of the banking modes.
#[derive(Clone, Debug)]
pub struct Pmu {
    pub spec: PmuSpec,
    pub mode: BankingMode,
    banks: Vec<Vec<i64>>,
    /// Under duplication every bank holds every word; otherwise each word lives in exactly one bank at
    /// offset `addr / banks` (strided/FIFO) or `addr / width`-derived (line buffer) — see [`Pmu::slot`].
    fifo_head: usize,
    fifo_tail: usize,
}

impl Pmu {
    pub fn new(spec: PmuSpec, mode: BankingMode) -> Pmu {
        Pmu {
            spec,
            mode,
            banks: vec![vec![0; spec.words_per_bank]; spec.banks],
            fifo_head: 0,
            fifo_tail: 0,
        }
    }

    /// `(bank, offset)` of a flat address under a non-duplicating mode: the bank from [`bank_of`], the
    /// offset = the address's rank among the addresses that share its bank — an injective map onto the
    /// bank's words for every address below the capacity.
    pub fn slot(&self, addr: usize) -> (usize, usize) {
        assert!(
            addr < self.spec.usable_words(self.mode),
            "address {addr} beyond the PMU's {} usable words under {:?}",
            self.spec.usable_words(self.mode),
            self.mode
        );
        let b = bank_of(addr, self.mode, self.spec.banks);
        let off = match self.mode {
            BankingMode::Strided { granularity } => {
                let g = granularity.max(1);
                (addr / (g * self.spec.banks)) * g + addr % g
            }
            BankingMode::Fifo => addr / self.spec.banks,
            BankingMode::LineBuffer { width } => {
                // Row `r`, column `c`: bank `c % banks`, and within the bank the row's `ceil(w/banks)` columns
                // are packed as `c / banks` — dense per bank, whole rows only (see `usable_words`).
                let w = width.max(1);
                let row = addr / w;
                let col = addr % w;
                row * ((w + self.spec.banks - 1) / self.spec.banks) + col / self.spec.banks
            }
            BankingMode::Duplication => addr,
        };
        (b, off)
    }

    pub fn write(&mut self, addr: usize, v: i64) {
        if self.mode == BankingMode::Duplication {
            assert!(
                addr < self.spec.words_per_bank,
                "duplicated address {addr} beyond one bank's {} words",
                self.spec.words_per_bank
            );
            for b in &mut self.banks {
                b[addr] = v;
            }
            return;
        }
        let (b, off) = self.slot(addr);
        self.banks[b][off] = v;
    }

    pub fn read(&self, addr: usize) -> i64 {
        if self.mode == BankingMode::Duplication {
            assert!(
                addr < self.spec.words_per_bank,
                "duplicated address {addr} beyond one bank's {} words",
                self.spec.words_per_bank
            );
            return self.banks[addr % self.spec.banks][addr];
        }
        let (b, off) = self.slot(addr);
        self.banks[b][off]
    }

    /// A vector read: the lane values and the cycles it costs (the bank conflicts).
    pub fn vector_read(&self, access: &AffineAccess) -> (Vec<i64>, usize) {
        (
            access.addrs().iter().map(|&a| self.read(a)).collect(),
            conflicts(access, self.mode, &self.spec),
        )
    }

    /// A vector write: cycles it costs (bank conflicts; a duplicated write is broadcast, one cycle per lane
    /// round since every bank must absorb every lane).
    pub fn vector_write(&mut self, access: &AffineAccess, vals: &[i64]) -> usize {
        for (a, v) in access.addrs().into_iter().zip(vals) {
            self.write(a, *v);
        }
        if self.mode == BankingMode::Duplication {
            access.lanes.max(1)
        } else {
            conflicts(access, self.mode, &self.spec)
        }
    }

    /// FIFO push (mode must be `Fifo`); returns false when full.
    pub fn push(&mut self, v: i64) -> bool {
        assert_eq!(self.mode, BankingMode::Fifo);
        if self.fifo_tail - self.fifo_head >= self.spec.capacity_words() {
            return false;
        }
        let a = self.fifo_tail % self.spec.capacity_words();
        self.write(a, v);
        self.fifo_tail += 1;
        true
    }

    /// FIFO pop in push order.
    pub fn pop(&mut self) -> Option<i64> {
        assert_eq!(self.mode, BankingMode::Fifo);
        if self.fifo_head == self.fifo_tail {
            return None;
        }
        let a = self.fifo_head % self.spec.capacity_words();
        self.fifo_head += 1;
        Some(self.read(a))
    }
}
