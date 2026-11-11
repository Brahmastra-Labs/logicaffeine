//! The CPU library: every kernel the CPU tier's link calls, one C++17 source the host's compiler builds beside the
//! program, each kernel in the instruction set its shim declares.
//!
//! A kernel is a space of units — a product's outputs, a norm's rows, an attention's (row, head) pairs, a quantize's
//! blocks, an elementwise op's values — and a function that computes any range of them. The program runs a kernel as
//! phases on the host's threads (`lk_phases`): the caller and `n − 1` workers take chunks of a phase's units from the
//! run's own counter in turn, and every unit of a phase is done before any of the next is taken. A fused op is one run
//! of the phases its parts' units make, each part's units computed by the same function the part runs alone, so a fused
//! op's every value is its parts' to the bit. A unit is one thread's, computed one way, so every result is the same at
//! every thread count, and a thread the system descheduled holds back only the chunk it holds (on this box, under its
//! neighbours' load, Llama-3.2-1B Q8_0 decode at 8 threads 0.3–0.6 tok/s by contiguous ranges, 2.5–3.2 by chunks;
//! llama.cpp's CPU backend 1.7–2.2 beside it).
//!
//! A product by checkpoint rows reads them in their block format in place and its activation quantized in one word a
//! 32-value block (`lk_quant_blocks`: the block's largest magnitude over 127, each value rounded to the nearest code,
//! ties to even), each weight block's dot exact in 32-bit integers by `vpdpbusd`, then scaled by both blocks' scales
//! into one `f32` sum an output, block after block. Its kernels take eight outputs a vector: a few rows (a decode step's)
//! read eight outputs' rows in place, a chunk of rows (a prefill's) a block of outputs staged in the core's cache with
//! each block's codes four to a lane, so one `vpdpbusd` advances eight outputs' dots — each lane computed alone and alike,
//! so a row of a chunk is the bits it is alone.

use crate::tensor_graph::symbols::ClassId;

/// The classes the CPU link takes whole: a fused op of one of them is one run of phases (its parts' units, each part
/// computed by its own kernel's function), a primitive one of the library's kernels, a placement a read or a write
/// through a view. The CPU shim's native answers.
pub const LINKED_WHOLE: &[ClassId] = &[
    ClassId::NormWeight,
    ClassId::AddNorm,
    ClassId::NormThenAdd,
    ClassId::NormScale,
    ClassId::NormWeightOffset,
    ClassId::NormQuant,
    ClassId::AddNormQuant,
    ClassId::DualNorm,
    ClassId::GatedNorm,
    ClassId::EmbedNorm,
    ClassId::EmbedScale,
    ClassId::GemvNormEpilogue,
    ClassId::PrefillNormEpilogues,
    ClassId::AttnResidualNorm,
    ClassId::RecomputeFanout,
    ClassId::QkNormRope,
    ClassId::QkNormRopeKvAppend,
    ClassId::RopeKvAppend,
    ClassId::RopeQkOneLaunch,
    ClassId::QkNormRopeGateSplit,
    ClassId::QkvHeadEpilogue,
    ClassId::HorizontalNormRopeInsert,
    ClassId::CrossLayerLaunch,
    ClassId::HostResidentKv,
    ClassId::HostLayersOnCpu,
    ClassId::KvCacheQuant,
    ClassId::HadamardRotation,
    ClassId::Mla,
    ClassId::MlaPrologue,
    ClassId::MlaUnabsorbInFold,
    ClassId::FlashDecode,
    ClassId::FlashPrefill,
    ClassId::AttnGqaRowPacking,
    ClassId::AttnLseMerge,
    ClassId::AttentionVariantsInline,
    ClassId::Softcap,
    ClassId::AttnOutputQuant,
    ClassId::AttnOutputGate,
    ClassId::RowWriteQuant,
    ClassId::GluActMul,
    ClassId::GluActMulQuant,
    ClassId::GemvGateUpGlu,
    ClassId::ElementwiseChains,
    ClassId::ElementwiseRegions,
    ClassId::ReluSqr,
    ClassId::GemmBias,
    ClassId::GemmScale,
    ClassId::MergedQkv,
    ClassId::MergedGateUp,
    ClassId::GemmDequantInkernel,
    ClassId::AffineInputFold,
    ClassId::GdnStep,
    ClassId::ConvBiasSilu,
    ClassId::ConvParts,
    ClassId::GdnGateProjections,
    ClassId::GdnGating,
    ClassId::GdnDecodeWhole,
    ClassId::GdnInputEpilogue,
    ClassId::GdnPostConvPrep,
    ClassId::ConvSiluL2norm,
    ClassId::GdnOutputQuant,
    ClassId::DeltaQkNorm,
    ClassId::RouterTopk,
    ClassId::GroupedTopk,
    ClassId::RouterSharedGateMerge,
    ClassId::MoeTokenSort,
    ClassId::MoeDirectPairs,
    ClassId::MoeGateUpAct,
    ClassId::MoeGatherInOperand,
    ClassId::MoeDownCombine,
    ClassId::MoeDeferredFinalize,
    ClassId::SharedExpertFusion,
    ClassId::MaskedExpertActivation,
    ClassId::FusedSampling,
    ClassId::CapArgmax,
    ClassId::GemvQuant,
    ClassId::ElementwiseQuant,
    ClassId::StridedRead,
    ClassId::RowWrite,
    ClassId::JoinInPlace,
    ClassId::StridedValues,
];

/// The positions an attention's split covers (`LK_ATTN_SPLIT` of the library): a row attending more is attended a split
/// at a time, the splits merged by log-sum-exp.
pub const ATTN_SPLIT: usize = 256;

/// The library for x86-64 with AVX2, FMA and AVX-VNNI (`-mavx2 -mfma -mavxvnni -mf16c`).
pub const LIBRARY_X86_AVX2_VNNI: &str = r#"
#include <immintrin.h>
#include <algorithm>
#include <atomic>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <initializer_list>
#include <thread>
#include <vector>

// The host's threads: the caller and n - 1 workers. run(phases, spaces, job, ctx) cuts each phase's [0, units) into
// chunks every participant takes in turn from the phase's own counter, calling job(ctx, phase, lo, hi) on each; a
// participant out of a phase's chunks waits until every one of them is done before it takes the next phase's, and run
// returns when the last phase's are — so a thread the system descheduled holds back only the chunk it holds, and the
// caller never waits for a worker to arrive. A run lives in one of `RING` slots: a worker holds a slot (its `refs`) before
// it checks the slot is its run's and only then takes chunks, and the caller closes a slot before it waits for its
// holders to leave and rewrites it, both orders sequentially consistent — so no worker late from one run ever takes a
// chunk of another. Idle workers spin, then yield.
typedef void (*lk_job)(void* ctx, int phase, int lo, int hi);
// A phase's space: its units, and the fewest a thread takes at once.
struct lk_space { long units; int grain; };
struct lk_pool {
  static constexpr int RING = 4, PHASES = 32;
  struct alignas(64) Slot {
    std::atomic<unsigned> run{0};
    std::atomic<int> refs{0};
    std::atomic<int> next[PHASES], done[PHASES];
    lk_job job = nullptr;
    void* ctx = nullptr;
    int phases = 0, total[PHASES], chunk[PHASES], chunks[PHASES];
  };
  int n;
  std::vector<std::thread> workers;
  Slot slots[RING];
  alignas(64) std::atomic<unsigned> gen{0};
  std::atomic<bool> quit{false};
  unsigned issued = 0;
  static void idle(int& spins){ if(spins++ < 4096) _mm_pause(); else std::this_thread::yield(); }
  explicit lk_pool(int threads) : n(threads < 1 ? 1 : threads) {
    for(int i = 1; i < n; ++i)
      workers.emplace_back([this]{
        unsigned seen = 0;
        for(;;){
          unsigned g;
          for(int spins = 0; (g = gen.load(std::memory_order_acquire)) == seen;){
            if(quit.load(std::memory_order_relaxed)) return;
            idle(spins);
          }
          seen = g;
          if(quit.load(std::memory_order_relaxed)) return;
          Slot& s = slots[g % RING];
          s.refs.fetch_add(1, std::memory_order_seq_cst);
          if(s.run.load(std::memory_order_seq_cst) == g) take(s);
          s.refs.fetch_sub(1, std::memory_order_release);
        }
      });
  }
  static void take(Slot& s){
    for(int p = 0; p < s.phases; ++p){
      for(;;){
        const int c = s.next[p].fetch_add(1, std::memory_order_acq_rel);
        if(c >= s.chunks[p]) break;
        const int lo = c * s.chunk[p], hi = lo + s.chunk[p] < s.total[p] ? lo + s.chunk[p] : s.total[p];
        s.job(s.ctx, p, lo, hi);
        s.done[p].fetch_add(1, std::memory_order_acq_rel);
      }
      if(p + 1 < s.phases) for(int spins = 0; s.done[p].load(std::memory_order_acquire) < s.chunks[p];) idle(spins);
    }
  }
  void run(int phases, const lk_space* sp, lk_job f, void* c){
    if(phases <= 0) return;
    if(phases > PHASES) std::abort();
    const unsigned g = ++issued;
    Slot& s = slots[g % RING];
    s.run.store(0, std::memory_order_seq_cst);
    for(int spins = 0; s.refs.load(std::memory_order_seq_cst) != 0;) idle(spins);
    s.job = f; s.ctx = c; s.phases = phases;
    for(int p = 0; p < phases; ++p){
      const int units = (int)sp[p].units, per = units / (8 * n), chunk = per > sp[p].grain ? per : sp[p].grain;
      s.total[p] = units; s.chunk[p] = chunk > 0 ? chunk : 1;
      s.chunks[p] = units > 0 ? (units + s.chunk[p] - 1) / s.chunk[p] : 0;
      s.next[p].store(0, std::memory_order_relaxed);
      s.done[p].store(0, std::memory_order_relaxed);
    }
    s.run.store(g, std::memory_order_seq_cst);
    gen.store(g, std::memory_order_release);
    take(s);
    for(int spins = 0; s.done[phases - 1].load(std::memory_order_acquire) < s.chunks[phases - 1];) idle(spins);
  }
  ~lk_pool(){
    quit.store(true, std::memory_order_relaxed);
    gen.fetch_add(1, std::memory_order_release);
    for(auto& w : workers) w.join();
  }
};

// A kernel's phases on the pool, f(phase, lo, hi) computing units [lo, hi) of a phase; on the caller alone where no
// phase has more than one chunk's units (a decode step's row ops) or the pool is one thread. A run holds at most
// `PHASES` phases: a kernel of more runs as consecutive runs, each a barrier after the last as its phases are.
template<class F> static void lk_phases(lk_pool& p, std::initializer_list<lk_space> sp, const F& f){
  bool alone = p.n == 1;
  if(!alone){ alone = true; for(const lk_space& s : sp) if(s.units > s.grain) alone = false; }
  if(alone){ int ph = 0; for(const lk_space& s : sp){ if(s.units > 0) f(ph, 0L, s.units); ++ph; } return; }
  struct G { const F* f; int base; };
  const int total = (int)sp.size();
  for(int base = 0; base < total; base += lk_pool::PHASES){
    G g{&f, base};
    p.run(std::min(lk_pool::PHASES, total - base), sp.begin() + base, [](void* c, int ph, int lo, int hi){ const G& g = *(const G*)c; (*g.f)(g.base + ph, (long)lo, (long)hi); }, &g);
  }
}

static inline float lk_hsum(__m256 v){
  __m128 s = _mm_add_ps(_mm256_castps256_ps128(v), _mm256_extractf128_ps(v, 1));
  s = _mm_add_ps(s, _mm_movehl_ps(s, s));
  s = _mm_add_ss(s, _mm_movehdup_ps(s));
  return _mm_cvtss_f32(s);
}
static inline float lk_f16(uint16_t h){ return _cvtsh_ss(h); }
static inline uint16_t lk_to_f16(float v){ return _cvtss_sh(v, _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC); }
// A tensor kept as binary16 (an attention's cache under the binary16 cache): each value written rounded to the nearest
// and read widened, through the same subscripts an f32 buffer takes.
struct lk_h16ref { uint16_t* p; operator float() const { return lk_f16(*p); } lk_h16ref& operator=(float v){ *p = lk_to_f16(v); return *this; } };
struct lk_h16buf { uint16_t* p = nullptr; lk_h16ref operator[](long i) const { return {p + i}; } };
// Eight values of a row from `f32` or binary16, as `f32`.
static inline __m256 lk_load8(const float* x){ return _mm256_loadu_ps(x); }
static inline __m256 lk_load8(const uint16_t* x){ return _mm256_cvtph_ps(_mm_loadu_si128((const __m128i*)x)); }
static inline float lk_load1(const float* x){ return *x; }
static inline float lk_load1(const uint16_t* x){ return lk_f16(*x); }

// A cache in int8 rows: every `unit` values' codes `q` under one binary16 scale `s` (`Keep::Int8Rows`), read from value
// `o` on. A read at value `at` is a view of the codes there under their unit's scale, its values the codes times it.
struct lk_i8rows { int8_t* q = nullptr; uint16_t* s = nullptr; long unit = 1; long o = 0; };
struct lk_i8view { const int8_t* q; float s; };
static inline lk_i8view operator+(lk_i8view v, long i){ return {v.q + i, v.s}; }
static inline lk_i8rows lk_i8from(lk_i8rows c, long o){ c.o += o; return c; }
static inline lk_i8view lk_cache_at(const lk_i8rows& c, long at){ return {c.q + c.o + at, lk_f16(c.s[(c.o + at) / c.unit])}; }
static inline const float* lk_cache_at(const float* c, long at){ return c + at; }
static inline const uint16_t* lk_cache_at(const uint16_t* c, long at){ return c + at; }
static inline __m256 lk_load8(lk_i8view v){ return _mm256_mul_ps(_mm256_cvtepi32_ps(_mm256_cvtepi8_epi32(_mm_loadl_epi64((const __m128i*)v.q))), _mm256_set1_ps(v.s)); }
static inline float lk_load1(lk_i8view v){ return (float)v.q[0] * v.s; }
// Value i of an int8 cache, unturned.
static inline float lk_i8get(const lk_i8rows& c, long i){ return lk_load1(lk_cache_at(c, i)); }
// The unit of `n` values at `x` kept in int8 rows at value `at` of `c`, as `Keep::keep_row` keeps it: taken in `f32`
// and every step after in `f64` — turned first, where `turned`, by the unnormalized Walsh–Hadamard transform in
// `numerics::hadamard`'s butterfly order — its largest magnitude as `f32` over 127 in `f32`, rounded to binary16, its
// scale; each code the value over the scale to the nearest, ties to even, within ±127, every one zero where the scale
// rounds to zero. Its codes and scale are `keep_row`'s bit for bit.
static void lk_keep_row(const float* x, long n, bool turned, const lk_i8rows& c, long at){
  thread_local std::vector<double> y;
  y.assign(x, x + n);
  if(turned)
    for(long h = 1; h < n; h *= 2)
      for(long st = 0; st < n; st += 2 * h)
        for(long j = st; j < st + h; ++j){ const double a = y[j], b = y[j + h]; y[j] = a + b; y[j + h] = a - b; }
  double amax = 0;
  for(long i = 0; i < n; ++i) amax = std::max(amax, std::fabs(y[i]));
  const uint16_t sh = lk_to_f16((float)amax / 127.f);
  const double sc = lk_f16(sh);
  c.s[at / c.unit] = sh;
  for(long i = 0; i < n; ++i) c.q[at + i] = sc == 0 ? 0 : (int8_t)std::min(127., std::max(-127., std::nearbyint(y[i] / sc)));
}
// Units [lo, hi) of `n` values each turned by the Walsh–Hadamard transform over `n` (a power of two), in `f32`:
// unit u of `per` a row at x + (u / per)·xp + (u % per)·n, written at y + (u / per)·yp + (u % per)·n.
static void lk_turn_units(const float* x, long xp, float* y, long yp, long n, long per, long lo, long hi){
  thread_local std::vector<float> t;
  t.resize(n);
  const float inv = 1.f / (float)n;
  for(long u = lo; u < hi; ++u){
    const float* xr = x + (u / per) * xp + (u % per) * n;
    for(long i = 0; i < n; ++i) t[i] = xr[i];
    for(long h = 1; h < n; h *= 2)
      for(long st = 0; st < n; st += 2 * h)
        for(long j = st; j < st + h; ++j){ const float a = t[j], b = t[j + h]; t[j] = a + b; t[j + h] = a - b; }
    float* yr = y + (u / per) * yp + (u % per) * n;
    for(long i = 0; i < n; ++i) yr[i] = t[i] * inv;
  }
}

// e^x for eight lanes, within 1.26 ulp of the exact value (one ulp of std::exp): x clamped to [−104, 88.7228]
// (past them e^x rounds to 0 and overflows), n = round(x / ln 2), the remainder r = x − n·ln 2 in two parts of ln 2
// (Cody–Waite) under Cephes' degree-6 polynomial for e^r, times 2^n as two exact powers of two (2^⌊n/2⌋ · 2^⌈n/2⌉).
static inline __m256 lk_exp8(__m256 x){
  x = _mm256_min_ps(_mm256_max_ps(x, _mm256_set1_ps(-104.f)), _mm256_set1_ps(88.72283935546875f));
  const __m256 n = _mm256_round_ps(_mm256_mul_ps(x, _mm256_set1_ps(1.44269504088896341f)), _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC);
  __m256 r = _mm256_fnmadd_ps(n, _mm256_set1_ps(0.693359375f), x);
  r = _mm256_fnmadd_ps(n, _mm256_set1_ps(-2.12194440e-4f), r);
  __m256 p = _mm256_set1_ps(1.9875691500e-4f);
  p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(1.3981999507e-3f));
  p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(8.3334519073e-3f));
  p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(4.1665795894e-2f));
  p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(1.6666665459e-1f));
  p = _mm256_fmadd_ps(p, r, _mm256_set1_ps(5.0000001201e-1f));
  p = _mm256_fmadd_ps(p, _mm256_mul_ps(r, r), _mm256_add_ps(r, _mm256_set1_ps(1.f)));
  const __m256i ni = _mm256_cvtps_epi32(n), lo = _mm256_srai_epi32(ni, 1), hi = _mm256_sub_epi32(ni, lo);
  const __m256 a = _mm256_castsi256_ps(_mm256_slli_epi32(_mm256_add_epi32(lo, _mm256_set1_epi32(127)), 23));
  const __m256 b = _mm256_castsi256_ps(_mm256_slli_epi32(_mm256_add_epi32(hi, _mm256_set1_epi32(127)), 23));
  return _mm256_mul_ps(_mm256_mul_ps(p, a), b);
}
// e^x for one value, the lanes' function.
static inline float lk_exp(float x){ return _mm256_cvtss_f32(lk_exp8(_mm256_set1_ps(x))); }

// Blocks [lo, hi) of rows of k values (row r at x + r·xp) quantized one word a 32-value block: d = the block's largest
// magnitude over 127, q = x / d rounded to the nearest code, ties to even (0 where the block is zero), and the block's
// code sum where `s` is given.
static void lk_quant_blocks(const float* x, long xp, int8_t* q, float* d, int* s, int k, long lo, long hi){
  const __m256 sign = _mm256_set1_ps(-0.0f);
  const int nb = k / 32;
  for(long u = lo; u < hi; ++u){
    const long r = u / nb, b = u % nb;
    const float* xb = x + r * xp + 32 * b;
    __m256 v[4], m = _mm256_setzero_ps();
    for(int i = 0; i < 4; ++i){ v[i] = _mm256_loadu_ps(xb + 8 * i); m = _mm256_max_ps(m, _mm256_andnot_ps(sign, v[i])); }
    __m128 h = _mm_max_ps(_mm256_castps256_ps128(m), _mm256_extractf128_ps(m, 1));
    h = _mm_max_ps(h, _mm_movehl_ps(h, h));
    h = _mm_max_ss(h, _mm_movehdup_ps(h));
    const float amax = _mm_cvtss_f32(h), sc = amax / 127.f, inv = amax > 0.f ? 127.f / amax : 0.f;
    d[r * nb + b] = sc;
    const __m256 iv = _mm256_set1_ps(inv);
    __m256i c[4];
    for(int i = 0; i < 4; ++i) c[i] = _mm256_cvtps_epi32(_mm256_round_ps(_mm256_mul_ps(v[i], iv), _MM_FROUND_TO_NEAREST_INT | _MM_FROUND_NO_EXC));
    if(s){
      const __m256i t = _mm256_add_epi32(_mm256_add_epi32(c[0], c[1]), _mm256_add_epi32(c[2], c[3]));
      __m128i h4 = _mm_add_epi32(_mm256_castsi256_si128(t), _mm256_extracti128_si256(t, 1));
      h4 = _mm_add_epi32(h4, _mm_shuffle_epi32(h4, 0x4e));
      h4 = _mm_add_epi32(h4, _mm_shuffle_epi32(h4, 0xb1));
      s[r * nb + b] = _mm_cvtsi128_si32(h4);
    }
    __m256i p = _mm256_packs_epi16(_mm256_packs_epi32(c[0], c[1]), _mm256_packs_epi32(c[2], c[3]));
    p = _mm256_permutevar8x32_epi32(p, _mm256_setr_epi32(0, 4, 1, 5, 2, 6, 3, 7));
    _mm256_storeu_si256((__m256i*)(q + r * k + 32 * b), p);
  }
}

// Products by checkpoint rows take eight outputs a vector, a lane each. A block's dot (a Q8_0 block's 32 values, a Q4_K
// sub-block's 32, a Q6_K half's 16 under one scale) is one exact 32-bit sum of its code products, then each output one
// f32 sum in block order: acc = fma(dot, d·dx, acc) for Q8_0; acc = fma(dot, d·sc·dx, acc), then acc = fma(dmin·m,
// −dx·Σx, acc) for Q4_K; acc = fma(dot, d·sc·dx, acc) a half for Q6_K. A row kernel (a few rows: a decode step's) reads
// eight outputs' rows in place and sums each output's vpdpbusd lanes exactly; a panel (a chunk's) stages a block of
// outputs eight to a group in the core's cache, each block's codes as vectors whose lane u holds four consecutive codes
// of output u, so one vpdpbusd by a broadcast of four activation codes advances eight outputs' dots, and passes a few
// activation rows at a time over the block. Each lane is computed alone and alike, so a row is the same bits through
// either kernel, in any chunk, on any thread.

// A vector the staging buffers hold, aligned.
struct alignas(32) lk_v256 { __m256i v; };
// Eight vectors of eight 32-bit values transposed in place.
static inline void lk_tr8(__m256i* r){
  __m256i t[8], u[8];
  for(int i = 0; i < 8; i += 2){ t[i] = _mm256_unpacklo_epi32(r[i], r[i + 1]); t[i + 1] = _mm256_unpackhi_epi32(r[i], r[i + 1]); }
  for(int i = 0; i < 8; i += 4){
    u[i] = _mm256_unpacklo_epi64(t[i], t[i + 2]); u[i + 1] = _mm256_unpackhi_epi64(t[i], t[i + 2]);
    u[i + 2] = _mm256_unpacklo_epi64(t[i + 1], t[i + 3]); u[i + 3] = _mm256_unpackhi_epi64(t[i + 1], t[i + 3]);
  }
  for(int i = 0; i < 4; ++i){ r[i] = _mm256_permute2x128_si256(u[i], u[i + 4], 0x20); r[i + 4] = _mm256_permute2x128_si256(u[i], u[i + 4], 0x31); }
}
// Lane u of h0 the sum of vector u's lanes 0–3, of h1 its lanes 4–7.
static inline void lk_halves8(const __m256i* v, __m256i& h0, __m256i& h1){
  const __m256i a = _mm256_hadd_epi32(_mm256_hadd_epi32(v[0], v[1]), _mm256_hadd_epi32(v[2], v[3]));
  const __m256i b = _mm256_hadd_epi32(_mm256_hadd_epi32(v[4], v[5]), _mm256_hadd_epi32(v[6], v[7]));
  h0 = _mm256_permute2x128_si256(a, b, 0x20); h1 = _mm256_permute2x128_si256(a, b, 0x31);
}
// Lane u the sum of vector u's lanes.
static inline __m256i lk_sum8(const __m256i* v){ __m256i h0, h1; lk_halves8(v, h0, h1); return _mm256_add_epi32(h0, h1); }
// Eight outputs' rows, the last repeated past `oo`.
static inline void lk_rows8(const uint8_t* w, long pitch, long o0, int oo, const uint8_t** wr){ for(int u = 0; u < 8; ++u) wr[u] = w + (o0 + (u < oo ? u : oo - 1)) * pitch; }
// Lanes [0, oo) of `v` written to y.
static inline void lk_put(float* y, __m256 v, int oo){ if(oo == 8) _mm256_storeu_ps(y, v); else { alignas(32) float t[8]; _mm256_store_ps(t, v); for(int u = 0; u < oo; ++u) y[u] = t[u]; } }
// The 16 bytes at `off` of eight outputs' rows as four vectors, lane u of vector i word i of output u's.
static inline void lk_words4(const uint8_t* const* wr, long off, __m256i* w){
  __m256i v[4];
  for(int u = 0; u < 4; ++u) v[u] = _mm256_inserti128_si256(_mm256_castsi128_si256(_mm_loadu_si128((const __m128i*)(wr[u] + off))), _mm_loadu_si128((const __m128i*)(wr[u + 4] + off)), 1);
  const __m256i t0 = _mm256_unpacklo_epi32(v[0], v[1]), t1 = _mm256_unpackhi_epi32(v[0], v[1]);
  const __m256i t2 = _mm256_unpacklo_epi32(v[2], v[3]), t3 = _mm256_unpackhi_epi32(v[2], v[3]);
  w[0] = _mm256_unpacklo_epi64(t0, t2); w[1] = _mm256_unpackhi_epi64(t0, t2); w[2] = _mm256_unpacklo_epi64(t1, t3); w[3] = _mm256_unpackhi_epi64(t1, t3);
}
// Eight binary16 values, one a lane's low 16 bits, as f32.
static inline __m256 lk_halfs(__m256i lo16){
  const __m256i p = _mm256_permute4x64_epi64(_mm256_packus_epi32(_mm256_and_si256(lo16, _mm256_set1_epi32(0xffff)), _mm256_setzero_si256()), 0xd8);
  return _mm256_cvtph_ps(_mm256_castsi256_si128(p));
}
// Byte i of each lane, unsigned.
static inline __m256 lk_byte(__m256i w, int i){ return _mm256_cvtepi32_ps(_mm256_and_si256(_mm256_srli_epi32(w, 8 * i), _mm256_set1_epi32(0xff))); }
// Byte i of each lane, signed.
static inline __m256 lk_sbyte(__m256i w, int i){ return _mm256_cvtepi32_ps(_mm256_srai_epi32(_mm256_slli_epi32(w, 24 - 8 * i), 24)); }
// A Q4_K block's d·sc and dmin·m of its eight sub-blocks for eight outputs (ggml's get_scale_min_k4, a word at a time).
static inline void lk_q4k_scales(const uint8_t* const* wr, long off, __m256* ds, __m256* dm){
  __m256i w[4]; lk_words4(wr, off, w);
  const __m256 d = lk_halfs(w[0]), mn = lk_halfs(_mm256_srli_epi32(w[0], 16));
  const __m256i k1 = _mm256_set1_epi32(0x3f3f3f3f), k2 = _mm256_set1_epi32(0x0f0f0f0f), k3 = _mm256_set1_epi32(0x03030303);
  const __m256i sl = _mm256_and_si256(w[1], k1), ml = _mm256_and_si256(w[2], k1);
  const __m256i sh = _mm256_or_si256(_mm256_and_si256(w[3], k2), _mm256_slli_epi32(_mm256_and_si256(_mm256_srli_epi32(w[1], 6), k3), 4));
  const __m256i mh = _mm256_or_si256(_mm256_and_si256(_mm256_srli_epi32(w[3], 4), k2), _mm256_slli_epi32(_mm256_and_si256(_mm256_srli_epi32(w[2], 6), k3), 4));
  for(int i = 0; i < 4; ++i){
    ds[i] = _mm256_mul_ps(d, lk_byte(sl, i)); dm[i] = _mm256_mul_ps(mn, lk_byte(ml, i));
    ds[4 + i] = _mm256_mul_ps(d, lk_byte(sh, i)); dm[4 + i] = _mm256_mul_ps(mn, lk_byte(mh, i));
  }
}
// A Q6_K block's d·sc of its sixteen 16-value scales for eight outputs.
static inline void lk_q6k_scales(const uint8_t* const* wr, long off, __m256* sl){
  __m256i w[4]; lk_words4(wr, off + 192, w);
  const __m256 d = _mm256_cvtph_ps(_mm_setr_epi16(*(const int16_t*)(wr[0] + off + 208), *(const int16_t*)(wr[1] + off + 208), *(const int16_t*)(wr[2] + off + 208), *(const int16_t*)(wr[3] + off + 208),
                                                  *(const int16_t*)(wr[4] + off + 208), *(const int16_t*)(wr[5] + off + 208), *(const int16_t*)(wr[6] + off + 208), *(const int16_t*)(wr[7] + off + 208)));
  for(int i = 0; i < 16; ++i) sl[i] = _mm256_mul_ps(d, lk_sbyte(w[i / 4], i % 4));
}
// A Q8_0 block's d for eight outputs.
static inline __m256 lk_q8_scales(const uint8_t* const* wr, long off){
  return _mm256_cvtph_ps(_mm_setr_epi16(*(const int16_t*)(wr[0] + off), *(const int16_t*)(wr[1] + off), *(const int16_t*)(wr[2] + off), *(const int16_t*)(wr[3] + off),
                                        *(const int16_t*)(wr[4] + off), *(const int16_t*)(wr[5] + off), *(const int16_t*)(wr[6] + off), *(const int16_t*)(wr[7] + off)));
}
// The outputs a panel stages at once for rows of k values (codes a byte a value, scales and staged sums beside them):
// up to 768 KB, a core's L2 beside the rows passed over it, a multiple of sixteen.
static inline int lk_block_outputs(int k){ const int ob = (int)(786432L / (k + k / 4)) & ~15; return ob < 16 ? 16 : ob > 64 ? 64 : ob; }
// The tiles of R rows a panel passes over a staged block: G groups of eight outputs at a time, the last groups one by one.
template<int R, int G, class T> static inline void lk_tiles(int groups, const T& tile){
  int g0 = 0;
  for(; g0 + G <= groups; g0 += G) tile.template run<G>(g0);
  for(; g0 < groups; ++g0) tile.template run<1>(g0);
}

// y[r][o] = Σ_b f16(w[o][b].d) · d[r][b] · Σ_i w[o][b].q[i] · q[r][b][i], for outputs [lo, hi) of a Q8_0 matrix read in
// place (34-byte blocks, rows `pitch` bytes apart).
struct lk_q8_job { const uint8_t* w; long pitch; const int8_t* q; const float* d; float* y; int rows, n, k; long ypitch; };
// A few rows (a decode step's): eight outputs at a time, each block's dot by vpdpbusd over the activation's magnitudes
// and the weights carrying its signs (a Q8_0 code is never −128, so the sign moves exactly), the lanes summed exactly.
static void lk_gemm_q8_0_rows(const lk_q8_job& j, long lo, long hi){
  const int nb = j.k / 32;
  for(int r0 = 0; r0 < j.rows; r0 += 4){
    const int rr = j.rows - r0 < 4 ? j.rows - r0 : 4;
    for(long o0 = lo; o0 < hi; o0 += 8){
      const int oo = hi - o0 < 8 ? (int)(hi - o0) : 8;
      const uint8_t* wr[8]; lk_rows8(j.w, j.pitch, o0, oo, wr);
      __m256 acc[4] = {_mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps()};
      for(int b = 0; b < nb; ++b){
        const __m256 dw = lk_q8_scales(wr, 34L * b);
        __m256i wv[8];
        for(int u = 0; u < 8; ++u) wv[u] = _mm256_loadu_si256((const __m256i*)(wr[u] + 34L * b + 2));
        for(int t = 0; t < rr; ++t){
          const __m256i xv = _mm256_loadu_si256((const __m256i*)(j.q + (long)(r0 + t) * j.k + 32 * b)), xa = _mm256_sign_epi8(xv, xv);
          __m256i dv[8];
          for(int u = 0; u < 8; ++u) dv[u] = _mm256_dpbusd_avx_epi32(_mm256_setzero_si256(), xa, _mm256_sign_epi8(wv[u], xv));
          acc[t] = _mm256_fmadd_ps(_mm256_cvtepi32_ps(lk_sum8(dv)), _mm256_mul_ps(dw, _mm256_set1_ps(j.d[(long)(r0 + t) * nb + b])), acc[t]);
        }
      }
      for(int t = 0; t < rr; ++t) lk_put(j.y + (long)(r0 + t) * j.ypitch + o0, acc[t], oo);
    }
  }
}
// R rows over G staged groups of a Q8_0 block of outputs.
template<int R>
struct lk_q8_tile {
  const lk_v256* codes; const lk_v256* neg; const float* dw; const uint8_t* const* xr; const float* const* dr; float* y; long ypitch; int nb, rr, bn; long o0;
  template<int G> void run(int g0) const {
    __m256 acc[R][G];
    for(int t = 0; t < R; ++t) for(int g = 0; g < G; ++g) acc[t][g] = _mm256_setzero_ps();
    for(int b = 0; b < nb; ++b){
      __m256i dv[R][G];
      for(int g = 0; g < G; ++g){ const __m256i n = neg[(size_t)(g0 + g) * nb + b].v; for(int t = 0; t < R; ++t) dv[t][g] = n; }
      for(int q = 0; q < 8; ++q){
        __m256i wv[G];
        for(int g = 0; g < G; ++g) wv[g] = codes[((size_t)(g0 + g) * nb + b) * 8 + q].v;
        for(int t = 0; t < R; ++t){
          int32_t x4; std::memcpy(&x4, xr[t] + 32L * b + 4 * q, 4);
          const __m256i xv = _mm256_set1_epi32(x4);
          for(int g = 0; g < G; ++g) dv[t][g] = _mm256_dpbusd_avx_epi32(dv[t][g], xv, wv[g]);
        }
      }
      for(int t = 0; t < R; ++t){
        const __m256 dx = _mm256_broadcast_ss(dr[t] + b);
        for(int g = 0; g < G; ++g)
          acc[t][g] = _mm256_fmadd_ps(_mm256_cvtepi32_ps(dv[t][g]), _mm256_mul_ps(_mm256_loadu_ps(dw + ((size_t)(g0 + g) * nb + b) * 8), dx), acc[t][g]);
      }
    }
    for(int t = 0; t < rr; ++t) for(int g = 0; g < G; ++g){ const int oo = bn - 8 * (g0 + g) < 8 ? bn - 8 * (g0 + g) : 8; lk_put(y + (long)t * ypitch + o0 + 8 * (g0 + g), acc[t][g], oo); }
  }
};
// Many rows (a prefill chunk's): a block of outputs' codes interleaved four to a lane and their scales staged once; each
// block's dot taken over the activation's codes plus 128 (unsigned, the sign bit flipped) from lanes that start at −128
// times the output's code sum, so exactly the row kernel's.
template<int R, int G>
static void lk_gemm_q8_0_panel(const lk_q8_job& j, long lo, long hi){
  const int nb = j.k / 32, ob = lk_block_outputs(j.k);
  const __m256i k128 = _mm256_set1_epi8((char)0x80);
  thread_local std::vector<lk_v256> codes, neg; thread_local std::vector<float> dw; thread_local std::vector<uint8_t> xu;
  codes.resize((size_t)ob / 8 * nb * 8); neg.resize((size_t)ob / 8 * nb); dw.resize((size_t)ob * nb); xu.resize((size_t)R * j.k);
  for(long b0 = lo; b0 < hi; b0 += ob){
    const int bn = hi - b0 < ob ? (int)(hi - b0) : ob, groups = (bn + 7) / 8;
    for(int g = 0; g < groups; ++g){
      const uint8_t* wr[8]; lk_rows8(j.w, j.pitch, b0 + 8 * g, bn - 8 * g < 8 ? bn - 8 * g : 8, wr);
      for(int b = 0; b < nb; ++b){
        __m256i v[8], s[8];
        for(int u = 0; u < 8; ++u){
          v[u] = _mm256_loadu_si256((const __m256i*)(wr[u] + 34L * b + 2));
          s[u] = _mm256_dpbusd_avx_epi32(_mm256_setzero_si256(), k128, v[u]);
        }
        neg[(size_t)g * nb + b].v = _mm256_sub_epi32(_mm256_setzero_si256(), lk_sum8(s));
        _mm256_storeu_ps(&dw[((size_t)g * nb + b) * 8], lk_q8_scales(wr, 34L * b));
        lk_tr8(v);
        for(int q = 0; q < 8; ++q) codes[((size_t)g * nb + b) * 8 + q].v = v[q];
      }
    }
    for(int r0 = 0; r0 < j.rows; r0 += R){
      const int rr = j.rows - r0 < R ? j.rows - r0 : R;
      const uint8_t* xr[R]; const float* dr[R];
      for(int t = 0; t < R; ++t){
        const long r = r0 + (t < rr ? t : rr - 1);
        uint8_t* x = &xu[(size_t)t * j.k];
        for(int i = 0; i < j.k; i += 32) _mm256_storeu_si256((__m256i*)(x + i), _mm256_xor_si256(_mm256_loadu_si256((const __m256i*)(j.q + r * j.k + i)), k128));
        xr[t] = x; dr[t] = j.d + r * nb;
      }
      lk_tiles<R, G>(groups, lk_q8_tile<R>{codes.data(), neg.data(), dw.data(), xr, dr, j.y + (long)r0 * j.ypitch, j.ypitch, nb, rr, bn, b0});
    }
  }
}
static void lk_gemm_q8_0_units(const lk_q8_job& j, long lo, long hi){ if(j.rows > 4) lk_gemm_q8_0_panel<3, 2>(j, lo, hi); else lk_gemm_q8_0_rows(j, lo, hi); }

// The precise numerics' product by Q8_0 rows read in place: y[r][o] = Σ_b f16(w[o][b].d) · Σ_i w[o][b].q[i] · x[r][i],
// each block's codes widened to f32 against the activation's own values (row r at x + r·xp), outputs [lo, hi).
struct lk_q8f_job { const uint8_t* w; long pitch; const float* x; long xp; float* y; int rows, n, k; long ypitch; };
static void lk_gemm_q8_0_f32_units(const lk_q8f_job& j, long lo, long hi){
  const int nb = j.k / 32;
  for(int r = 0; r < j.rows; ++r)
    for(long o = lo; o < hi; ++o){
      const uint8_t* wr = j.w + o * j.pitch;
      const float* xr = j.x + (long)r * j.xp;
      __m256 acc = _mm256_setzero_ps();
      for(int b = 0; b < nb; ++b){
        uint16_t dh; std::memcpy(&dh, wr + 34 * b, 2);
        const __m256 d = _mm256_set1_ps(lk_f16(dh));
        const __m128i* q = (const __m128i*)(wr + 34 * b + 2);
        const __m128i lo16 = _mm_loadu_si128(q), hi16 = _mm_loadu_si128(q + 1);
        const __m256i c[4] = {_mm256_cvtepi8_epi32(lo16), _mm256_cvtepi8_epi32(_mm_srli_si128(lo16, 8)),
                              _mm256_cvtepi8_epi32(hi16), _mm256_cvtepi8_epi32(_mm_srli_si128(hi16, 8))};
        __m256 blk = _mm256_setzero_ps();
        for(int i = 0; i < 4; ++i) blk = _mm256_fmadd_ps(_mm256_cvtepi32_ps(c[i]), _mm256_loadu_ps(xr + 32 * b + 8 * i), blk);
        acc = _mm256_fmadd_ps(blk, d, acc);
      }
      j.y[(long)r * j.ypitch + o] = lk_hsum(acc);
    }
}

// The 6-bit scale and minimum of sub-block j of a K-quant's 12 packed bytes (ggml's get_scale_min_k4).
static inline void lk_scale_min_k4(int j, const uint8_t* q, int& sc, int& m){
  if(j < 4){ sc = q[j] & 63; m = q[j + 4] & 63; }
  else { sc = (q[j + 4] & 0xf) | ((q[j - 4] >> 6) << 4); m = (q[j + 4] >> 4) | ((q[j] >> 6) << 4); }
}
// A 256-value K-quant block dequantized: Q4_K (144 bytes) and Q6_K (210), as ggml lays them out.
static void lk_deq_q4_k(const uint8_t* b, float* y){
  uint16_t dh, mh; std::memcpy(&dh, b, 2); std::memcpy(&mh, b + 2, 2);
  const float d = lk_f16(dh), mn = lk_f16(mh);
  const uint8_t* sc = b + 4; const uint8_t* qs = b + 16;
  for(int j = 0, is = 0; j < 256; j += 64, is += 2){
    int s1, m1, s2, m2; lk_scale_min_k4(is, sc, s1, m1); lk_scale_min_k4(is + 1, sc, s2, m2);
    const uint8_t* q = qs + j / 2;
    for(int l = 0; l < 32; ++l){ y[j + l] = d * s1 * (q[l] & 0xf) - mn * m1; y[j + 32 + l] = d * s2 * (q[l] >> 4) - mn * m2; }
  }
}
static void lk_deq_q6_k(const uint8_t* b, float* y){
  uint16_t dh; std::memcpy(&dh, b + 208, 2);
  const float d = lk_f16(dh);
  const int8_t* sc = (const int8_t*)(b + 192);
  for(int part = 0; part < 2; ++part){
    const uint8_t* ql = b + 64 * part; const uint8_t* qh = b + 128 + 32 * part; const int8_t* s = sc + 8 * part; float* o = y + 128 * part;
    for(int l = 0; l < 32; ++l){
      const int is = l / 16;
      o[l] = d * s[is] * ((int)((ql[l] & 0xf) | ((qh[l] & 3) << 4)) - 32);
      o[l + 32] = d * s[is + 2] * ((int)((ql[l + 32] & 0xf) | (((qh[l] >> 2) & 3) << 4)) - 32);
      o[l + 64] = d * s[is + 4] * ((int)((ql[l] >> 4) | (((qh[l] >> 4) & 3) << 4)) - 32);
      o[l + 96] = d * s[is + 6] * ((int)((ql[l + 32] >> 4) | (((qh[l] >> 6) & 3) << 4)) - 32);
    }
  }
}
// A product by K-quant rows read in place — F 12 Q4_K, 14 Q6_K — its activation one word a block (`q`, `d` and the
// blocks' code sums `s`) or its own values (`x`, row r at x + r·xp, the precise numerics).
struct lk_kq_job { int f; const uint8_t* w; long pitch; const int8_t* q; const float* d; const int* s; const float* x; long xp; float* y; int rows, n, k; long ypitch; };
// Q4_K, a few rows: eight outputs at a time, each sub-block's eight dots by vpdpbusd over the row's codes (unsigned
// nibbles against the activation's codes), the lanes summed exactly; a block's scales and minimums taken once for every
// row.
static void lk_gemm_q4_k_rows(const lk_kq_job& j, long lo, long hi){
  const int nb = j.k / 256, xb = j.k / 32;
  const __m256i lowm = _mm256_set1_epi8(0x0f);
  for(int r0 = 0; r0 < j.rows; r0 += 4){
    const int rr = j.rows - r0 < 4 ? j.rows - r0 : 4;
    for(long o0 = lo; o0 < hi; o0 += 8){
      const int oo = hi - o0 < 8 ? (int)(hi - o0) : 8;
      const uint8_t* wr[8]; lk_rows8(j.w, j.pitch, o0, oo, wr);
      __m256 acc[4] = {_mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps()};
      for(int b = 0; b < nb; ++b){
        __m256 ds[8], dm[8]; lk_q4k_scales(wr, 144L * b, ds, dm);
        for(int p = 0; p < 4; ++p){
          __m256i cl[8], ch[8];
          for(int u = 0; u < 8; ++u){
            const __m256i qv = _mm256_loadu_si256((const __m256i*)(wr[u] + 144L * b + 16 + 32 * p));
            cl[u] = _mm256_and_si256(qv, lowm); ch[u] = _mm256_and_si256(_mm256_srli_epi16(qv, 4), lowm);
          }
          for(int t = 0; t < rr; ++t)
            for(int h = 0; h < 2; ++h){
              const long ab = (long)(r0 + t) * xb + 8 * b + 2 * p + h;
              const __m256i xv = _mm256_loadu_si256((const __m256i*)(j.q + 32 * ab));
              __m256i dv[8];
              for(int u = 0; u < 8; ++u) dv[u] = _mm256_dpbusd_avx_epi32(_mm256_setzero_si256(), h ? ch[u] : cl[u], xv);
              acc[t] = _mm256_fmadd_ps(_mm256_cvtepi32_ps(lk_sum8(dv)), _mm256_mul_ps(ds[2 * p + h], _mm256_set1_ps(j.d[ab])), acc[t]);
              acc[t] = _mm256_fmadd_ps(dm[2 * p + h], _mm256_set1_ps(-(j.d[ab] * (float)j.s[ab])), acc[t]);
            }
        }
      }
      for(int t = 0; t < rr; ++t) lk_put(j.y + (long)(r0 + t) * j.ypitch + o0, acc[t], oo);
    }
  }
}
// R rows over G staged groups of a Q4_K block of outputs.
template<int R>
struct lk_q4k_tile {
  const lk_v256* codes; const float* ds; const float* dm; const int8_t* const* xr; const float* const* dr; const float* nax; float* y; long ypitch; int ns, rr, bn; long o0;
  template<int G> void run(int g0) const {
    __m256 acc[R][G];
    for(int t = 0; t < R; ++t) for(int g = 0; g < G; ++g) acc[t][g] = _mm256_setzero_ps();
    for(int sb = 0; sb < ns; ++sb){
      __m256i dv[R][G];
      for(int t = 0; t < R; ++t) for(int g = 0; g < G; ++g) dv[t][g] = _mm256_setzero_si256();
      for(int q = 0; q < 8; ++q){
        __m256i wv[G];
        for(int g = 0; g < G; ++g) wv[g] = codes[((size_t)(g0 + g) * ns + sb) * 8 + q].v;
        for(int t = 0; t < R; ++t){
          int32_t x4; std::memcpy(&x4, xr[t] + 32L * sb + 4 * q, 4);
          const __m256i xv = _mm256_set1_epi32(x4);
          for(int g = 0; g < G; ++g) dv[t][g] = _mm256_dpbusd_avx_epi32(dv[t][g], wv[g], xv);
        }
      }
      for(int t = 0; t < R; ++t){
        const __m256 dx = _mm256_broadcast_ss(dr[t] + sb), nx = _mm256_broadcast_ss(nax + (size_t)t * ns + sb);
        for(int g = 0; g < G; ++g){
          const size_t at = ((size_t)(g0 + g) * ns + sb) * 8;
          acc[t][g] = _mm256_fmadd_ps(_mm256_cvtepi32_ps(dv[t][g]), _mm256_mul_ps(_mm256_loadu_ps(ds + at), dx), acc[t][g]);
          acc[t][g] = _mm256_fmadd_ps(_mm256_loadu_ps(dm + at), nx, acc[t][g]);
        }
      }
    }
    for(int t = 0; t < rr; ++t) for(int g = 0; g < G; ++g){ const int oo = bn - 8 * (g0 + g) < 8 ? bn - 8 * (g0 + g) : 8; lk_put(y + (long)t * ypitch + o0 + 8 * (g0 + g), acc[t][g], oo); }
  }
};
// Q4_K, many rows: a block of outputs' codes unpacked and interleaved four to a lane, their sub-blocks' d·sc and dmin·m
// staged once a vector; every R rows passed over the block, each row's −dx·Σx taken once.
template<int R, int G>
static void lk_gemm_q4_k_panel(const lk_kq_job& j, long lo, long hi){
  const int nb = j.k / 256, ns = j.k / 32, ob = lk_block_outputs(j.k);
  const __m256i lowm = _mm256_set1_epi8(0x0f);
  thread_local std::vector<lk_v256> codes; thread_local std::vector<float> ds, dm, nax;
  codes.resize((size_t)ob / 8 * ns * 8); ds.resize((size_t)ob * ns); dm.resize((size_t)ob * ns); nax.resize((size_t)R * ns);
  for(long b0 = lo; b0 < hi; b0 += ob){
    const int bn = hi - b0 < ob ? (int)(hi - b0) : ob, groups = (bn + 7) / 8;
    for(int g = 0; g < groups; ++g){
      const uint8_t* wr[8]; lk_rows8(j.w, j.pitch, b0 + 8 * g, bn - 8 * g < 8 ? bn - 8 * g : 8, wr);
      for(int b = 0; b < nb; ++b){
        __m256 sd[8], sm[8]; lk_q4k_scales(wr, 144L * b, sd, sm);
        for(int i = 0; i < 8; ++i){ _mm256_storeu_ps(&ds[((size_t)g * ns + 8 * b + i) * 8], sd[i]); _mm256_storeu_ps(&dm[((size_t)g * ns + 8 * b + i) * 8], sm[i]); }
        for(int p = 0; p < 4; ++p){
          __m256i l[8], h[8];
          for(int u = 0; u < 8; ++u){
            const __m256i qv = _mm256_loadu_si256((const __m256i*)(wr[u] + 144L * b + 16 + 32 * p));
            l[u] = _mm256_and_si256(qv, lowm); h[u] = _mm256_and_si256(_mm256_srli_epi16(qv, 4), lowm);
          }
          lk_tr8(l); lk_tr8(h);
          lk_v256* c0 = &codes[((size_t)g * ns + 8 * b + 2 * p) * 8];
          for(int q = 0; q < 8; ++q){ c0[q].v = l[q]; c0[8 + q].v = h[q]; }
        }
      }
    }
    for(int r0 = 0; r0 < j.rows; r0 += R){
      const int rr = j.rows - r0 < R ? j.rows - r0 : R;
      const int8_t* xr[R]; const float* dr[R];
      for(int t = 0; t < R; ++t){
        const long a = (long)(r0 + (t < rr ? t : rr - 1)) * ns;
        xr[t] = j.q + 32 * a; dr[t] = j.d + a;
        for(int sb = 0; sb < ns; ++sb) nax[(size_t)t * ns + sb] = -(j.d[a + sb] * (float)j.s[a + sb]);
      }
      lk_tiles<R, G>(groups, lk_q4k_tile<R>{codes.data(), ds.data(), dm.data(), xr, dr, nax.data(), j.y + (long)r0 * j.ypitch, j.ypitch, ns, rr, bn, b0});
    }
  }
}

// A Q6_K block half's four 32-value sub-blocks of 6-bit codes (unsigned, the value plus 32).
static inline void lk_q6_unpack(const uint8_t* blk, int part, __m256i* u){
  const __m256i m4 = _mm256_set1_epi8(0x0f), m2 = _mm256_set1_epi8(3);
  const __m256i ql0 = _mm256_loadu_si256((const __m256i*)(blk + 64 * part)), ql1 = _mm256_loadu_si256((const __m256i*)(blk + 64 * part + 32));
  const __m256i qh = _mm256_loadu_si256((const __m256i*)(blk + 128 + 32 * part));
  u[0] = _mm256_or_si256(_mm256_and_si256(ql0, m4), _mm256_slli_epi16(_mm256_and_si256(qh, m2), 4));
  u[1] = _mm256_or_si256(_mm256_and_si256(ql1, m4), _mm256_slli_epi16(_mm256_and_si256(_mm256_srli_epi16(qh, 2), m2), 4));
  u[2] = _mm256_or_si256(_mm256_and_si256(_mm256_srli_epi16(ql0, 4), m4), _mm256_slli_epi16(_mm256_and_si256(_mm256_srli_epi16(qh, 4), m2), 4));
  u[3] = _mm256_or_si256(_mm256_and_si256(_mm256_srli_epi16(ql1, 4), m4), _mm256_slli_epi16(_mm256_and_si256(_mm256_srli_epi16(qh, 6), m2), 4));
}
// −32 times the sum of each 16 of a row's codes.
static inline void lk_q6_offsets(const int8_t* x, int halves, int32_t* o){
  for(int h = 0; h < halves; ++h){
    const __m128i v = _mm_loadu_si128((const __m128i*)(x + 16 * h));
    __m128i s = _mm_sad_epu8(_mm_xor_si128(v, _mm_set1_epi8((char)0x80)), _mm_setzero_si128());
    s = _mm_add_epi64(s, _mm_unpackhi_epi64(s, s));
    o[h] = -32 * (_mm_cvtsi128_si32(s) - 16 * 128);
  }
}
// Q6_K, a few rows: eight outputs at a time, each sub-block's eight dots by vpdpbusd over the row's codes, each half's
// lanes summed exactly and −32 times its activation sum added.
static void lk_gemm_q6_k_rows(const lk_kq_job& j, long lo, long hi){
  const int nb = j.k / 256, xb = j.k / 32;
  thread_local std::vector<int32_t> off; off.resize((size_t)4 * 2 * xb);
  for(int r0 = 0; r0 < j.rows; r0 += 4){
    const int rr = j.rows - r0 < 4 ? j.rows - r0 : 4;
    for(int t = 0; t < rr; ++t) lk_q6_offsets(j.q + 32L * (r0 + t) * xb, 2 * xb, &off[(size_t)t * 2 * xb]);
    for(long o0 = lo; o0 < hi; o0 += 8){
      const int oo = hi - o0 < 8 ? (int)(hi - o0) : 8;
      const uint8_t* wr[8]; lk_rows8(j.w, j.pitch, o0, oo, wr);
      __m256 acc[4] = {_mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps(), _mm256_setzero_ps()};
      for(int b = 0; b < nb; ++b){
        __m256 sl[16]; lk_q6k_scales(wr, 210L * b, sl);
        for(int part = 0; part < 2; ++part){
          __m256i cu[8][4];
          for(int u = 0; u < 8; ++u) lk_q6_unpack(wr[u] + 210L * b, part, cu[u]);
          for(int t = 0; t < rr; ++t)
            for(int q = 0; q < 4; ++q){
              const int sb = 8 * b + 4 * part + q;
              const long ab = (long)(r0 + t) * xb + sb;
              const __m256i xv = _mm256_loadu_si256((const __m256i*)(j.q + 32 * ab));
              __m256i dv[8];
              for(int u = 0; u < 8; ++u) dv[u] = _mm256_dpbusd_avx_epi32(_mm256_setzero_si256(), cu[u][q], xv);
              __m256i h[2]; lk_halves8(dv, h[0], h[1]);
              const __m256 dx = _mm256_set1_ps(j.d[ab]);
              for(int hh = 0; hh < 2; ++hh)
                acc[t] = _mm256_fmadd_ps(_mm256_cvtepi32_ps(_mm256_add_epi32(h[hh], _mm256_set1_epi32(off[(size_t)t * 2 * xb + 2 * sb + hh]))), _mm256_mul_ps(sl[8 * part + 2 * q + hh], dx), acc[t]);
            }
        }
      }
      for(int t = 0; t < rr; ++t) lk_put(j.y + (long)(r0 + t) * j.ypitch + o0, acc[t], oo);
    }
  }
}
// R rows over G staged groups of a Q6_K block of outputs: each half's dot from −32 times its activation sum.
template<int R>
struct lk_q6k_tile {
  const lk_v256* codes; const float* sl; const int8_t* const* xr; const float* const* dr; const int32_t* off; float* y; long ypitch; int ns, rr, bn; long o0;
  template<int G> void run(int g0) const {
    __m256 acc[R][G];
    for(int t = 0; t < R; ++t) for(int g = 0; g < G; ++g) acc[t][g] = _mm256_setzero_ps();
    for(int sb = 0; sb < ns; ++sb)
      for(int hh = 0; hh < 2; ++hh){
        __m256i dv[R][G];
        for(int t = 0; t < R; ++t){ const __m256i o = _mm256_set1_epi32(off[(size_t)t * 2 * ns + 2 * sb + hh]); for(int g = 0; g < G; ++g) dv[t][g] = o; }
        for(int q = 4 * hh; q < 4 * hh + 4; ++q){
          __m256i wv[G];
          for(int g = 0; g < G; ++g) wv[g] = codes[((size_t)(g0 + g) * ns + sb) * 8 + q].v;
          for(int t = 0; t < R; ++t){
            int32_t x4; std::memcpy(&x4, xr[t] + 32L * sb + 4 * q, 4);
            const __m256i xv = _mm256_set1_epi32(x4);
            for(int g = 0; g < G; ++g) dv[t][g] = _mm256_dpbusd_avx_epi32(dv[t][g], wv[g], xv);
          }
        }
        for(int t = 0; t < R; ++t){
          const __m256 dx = _mm256_broadcast_ss(dr[t] + sb);
          for(int g = 0; g < G; ++g)
            acc[t][g] = _mm256_fmadd_ps(_mm256_cvtepi32_ps(dv[t][g]), _mm256_mul_ps(_mm256_loadu_ps(sl + (((size_t)(g0 + g) * ns + sb) * 2 + hh) * 8), dx), acc[t][g]);
        }
      }
    for(int t = 0; t < rr; ++t) for(int g = 0; g < G; ++g){ const int oo = bn - 8 * (g0 + g) < 8 ? bn - 8 * (g0 + g) : 8; lk_put(y + (long)t * ypitch + o0 + 8 * (g0 + g), acc[t][g], oo); }
  }
};
// Q6_K, many rows: a block of outputs' 6-bit codes interleaved four to a lane and their halves' d·sc staged once.
template<int R, int G>
static void lk_gemm_q6_k_panel(const lk_kq_job& j, long lo, long hi){
  const int nb = j.k / 256, ns = j.k / 32, ob = lk_block_outputs(j.k);
  thread_local std::vector<lk_v256> codes; thread_local std::vector<float> sl; thread_local std::vector<int32_t> off;
  codes.resize((size_t)ob / 8 * ns * 8); sl.resize((size_t)ob * ns * 2); off.resize((size_t)R * 2 * ns);
  for(long b0 = lo; b0 < hi; b0 += ob){
    const int bn = hi - b0 < ob ? (int)(hi - b0) : ob, groups = (bn + 7) / 8;
    for(int g = 0; g < groups; ++g){
      const uint8_t* wr[8]; lk_rows8(j.w, j.pitch, b0 + 8 * g, bn - 8 * g < 8 ? bn - 8 * g : 8, wr);
      for(int b = 0; b < nb; ++b){
        __m256 sc[16]; lk_q6k_scales(wr, 210L * b, sc);
        for(int part = 0; part < 2; ++part){
          __m256i cu[8][4];
          for(int u = 0; u < 8; ++u) lk_q6_unpack(wr[u] + 210L * b, part, cu[u]);
          for(int q = 0; q < 4; ++q){
            const int sb = 8 * b + 4 * part + q;
            __m256i v[8]; for(int u = 0; u < 8; ++u) v[u] = cu[u][q];
            lk_tr8(v);
            for(int k = 0; k < 8; ++k) codes[((size_t)g * ns + sb) * 8 + k].v = v[k];
            for(int hh = 0; hh < 2; ++hh) _mm256_storeu_ps(&sl[(((size_t)g * ns + sb) * 2 + hh) * 8], sc[8 * part + 2 * q + hh]);
          }
        }
      }
    }
    for(int r0 = 0; r0 < j.rows; r0 += R){
      const int rr = j.rows - r0 < R ? j.rows - r0 : R;
      const int8_t* xr[R]; const float* dr[R];
      for(int t = 0; t < R; ++t){
        const long a = (long)(r0 + (t < rr ? t : rr - 1)) * ns;
        xr[t] = j.q + 32 * a; dr[t] = j.d + a;
        lk_q6_offsets(xr[t], 2 * ns, &off[(size_t)t * 2 * ns]);
      }
      lk_tiles<R, G>(groups, lk_q6k_tile<R>{codes.data(), sl.data(), xr, dr, off.data(), j.y + (long)r0 * j.ypitch, j.ypitch, ns, rr, bn, b0});
    }
  }
}

// The precise numerics' products by K-quant rows: each 256-value block dequantized, then dotted with the activation's
// own values.
static void lk_gemm_kq_f32_units(const lk_kq_job& j, long lo, long hi){
  const int nb = j.k / 256;
  const long bb = j.f == 12 ? 144 : 210;
  alignas(32) float t[256];
  for(int r = 0; r < j.rows; ++r)
    for(long o = lo; o < hi; ++o){
      const uint8_t* wr = j.w + o * j.pitch;
      const float* xr = j.x + (long)r * j.xp;
      __m256 acc = _mm256_setzero_ps();
      for(int b = 0; b < nb; ++b){
        if(j.f == 12) lk_deq_q4_k(wr + bb * b, t); else lk_deq_q6_k(wr + bb * b, t);
        for(int i = 0; i < 256; i += 8) acc = _mm256_fmadd_ps(_mm256_load_ps(t + i), _mm256_loadu_ps(xr + 256 * b + i), acc);
      }
      j.y[(long)r * j.ypitch + o] = lk_hsum(acc);
    }
}
static void lk_gemm_kq_units(const lk_kq_job& j, long lo, long hi){
  if(!j.q) lk_gemm_kq_f32_units(j, lo, hi);
  else if(j.f == 12){ if(j.rows > 4) lk_gemm_q4_k_panel<3, 2>(j, lo, hi); else lk_gemm_q4_k_rows(j, lo, hi); }
  else { if(j.rows > 4) lk_gemm_q6_k_panel<2, 2>(j, lo, hi); else lk_gemm_q6_k_rows(j, lo, hi); }
}

// y[r][o] = Σ_i x[r][i] · w[o][i] for rows of f32 values `wp` floats apart (checkpoint rows in f32, a matrix of values
// laid out by output), outputs [lo, hi).
struct lk_f32_job { const float* w; long wp; const float* x; long xp; float* y; int rows, n, k; long ypitch; };
static void lk_gemm_f32_units(const lk_f32_job& j, long lo, long hi){
  for(int r = 0; r < j.rows; ++r)
    for(long o = lo; o < hi; ++o){
      const float* wr = j.w + o * j.wp;
      const float* xr = j.x + (long)r * j.xp;
      __m256 acc = _mm256_setzero_ps();
      int i = 0;
      for(; i + 8 <= j.k; i += 8) acc = _mm256_fmadd_ps(_mm256_loadu_ps(wr + i), _mm256_loadu_ps(xr + i), acc);
      float s = lk_hsum(acc);
      for(; i < j.k; ++i) s += wr[i] * xr[i];
      j.y[(long)r * j.ypitch + o] = s;
    }
}
// y[r][o] = Σ_i x[r][i] · b[i][o] for a matrix of values in the program's layout (`k × n`), outputs [lo, hi).
static void lk_gemm_vals_units(const float* x, long xp, const float* b, float* y, long ypitch, int rows, int n, int k, long lo, long hi){
  for(int r = 0; r < rows; ++r)
    for(long o = lo; o < hi; ++o){
      float s = 0.f;
      for(int i = 0; i < k; ++i) s += x[(long)r * xp + i] * b[(long)i * n + o];
      y[(long)r * ypitch + o] = s;
    }
}

// Rows [lo, hi) of a root mean square norm: y = (x + res) / sqrt(mean((x + res)^2) + eps) · w[r mod wrows] (row r of x at
// x + r·xp, of res at res + r·cols, of y at y + r·yp; no weight where `w` is null).
static void lk_rmsnorm_rows(const float* x, long xp, const float* res, const float* w, int wrows, float* y, long yp, int cols, float eps, long lo, long hi){
  for(long r = lo; r < hi; ++r){
    const float* xr = x + r * xp;
    const float* rr = res ? res + r * cols : nullptr;
    __m256 acc = _mm256_setzero_ps();
    int i = 0;
    for(; i + 8 <= cols; i += 8){
      __m256 v = _mm256_loadu_ps(xr + i);
      if(rr) v = _mm256_add_ps(v, _mm256_loadu_ps(rr + i));
      acc = _mm256_fmadd_ps(v, v, acc);
    }
    float ss = lk_hsum(acc);
    for(; i < cols; ++i){ const float v = xr[i] + (rr ? rr[i] : 0.f); ss += v * v; }
    const float inv = 1.f / std::sqrt(ss / cols + eps);
    const float* wr = w ? w + (r % (wrows < 1 ? 1 : wrows)) * cols : nullptr;
    float* yr = y + r * yp;
    for(int c = 0; c < cols; ++c){
      float v = (xr[c] + (rr ? rr[c] : 0.f)) * inv;
      if(wr) v *= wr[c];
      yr[c] = v;
    }
  }
}
// Rows [lo, hi) of a layer norm: y = (x − mean) / sqrt(var + eps) · w[r mod wrows] + b.
static void lk_layernorm_rows(const float* x, long xp, const float* w, int wrows, const float* b, float* y, long yp, int cols, float eps, long lo, long hi){
  for(long r = lo; r < hi; ++r){
    const float* xr = x + r * xp;
    float sm = 0.f;
    for(int c = 0; c < cols; ++c) sm += xr[c];
    const float mean = sm / cols;
    float vs = 0.f;
    for(int c = 0; c < cols; ++c) vs += (xr[c] - mean) * (xr[c] - mean);
    const float inv = 1.f / std::sqrt(vs / cols + eps);
    const float* wr = w + (r % (wrows < 1 ? 1 : wrows)) * cols;
    for(int c = 0; c < cols; ++c) y[r * yp + c] = (xr[c] - mean) * inv * wr[c] + b[c];
  }
}
// Rows [lo, hi) of a softmax.
static void lk_softmax_rows(const float* x, long xp, float* y, long yp, int cols, long lo, long hi){
  for(long r = lo; r < hi; ++r){
    const float* xr = x + r * xp;
    float m = xr[0];
    for(int c = 1; c < cols; ++c) m = xr[c] > m ? xr[c] : m;
    float s = 0.f;
    for(int c = 0; c < cols; ++c){ const float e = lk_exp(xr[c] - m); y[r * yp + c] = e; s += e; }
    for(int c = 0; c < cols; ++c) y[r * yp + c] /= s;
  }
}
// Rows [lo, hi) of an L2 norm: y = x / sqrt(Σ x^2 + eps).
static void lk_l2norm_rows(const float* x, long xp, float* y, long yp, int cols, float eps, long lo, long hi){
  for(long r = lo; r < hi; ++r){
    const float* xr = x + r * xp;
    float ss = 0.f;
    for(int c = 0; c < cols; ++c) ss += xr[c] * xr[c];
    const float den = std::sqrt(ss + eps);
    for(int c = 0; c < cols; ++c) y[r * yp + c] = xr[c] / den;
  }
}

// Head rows [lo, hi) rotated: each head row's pairs (i, i + half) by its position's angles — tables `cs`, `sn` of `half`
// a row, head row rh taking table row `first + rh / per_row` (`first` alone where `by_row` is false) — the rest copied;
// head row rh read at x + rh·xh and written at y + rh·yh.
static void lk_rope_units(const float* x, long xh, const float* cs, const float* sn, float* y, long yh, int hd, int half, int per_row, long first, bool by_row, long lo, long hi){
  for(long rh = lo; rh < hi; ++rh){
    const long t = (first + (by_row ? rh / per_row : 0)) * half;
    const float* xr = x + rh * xh;
    float* yr = y + rh * yh;
    for(int i = 0; i < half; ++i){
      const float a = xr[i], b = xr[i + half];
      yr[i] = a * cs[t + i] - b * sn[t + i];
      yr[i + half] = a * sn[t + i] + b * cs[t + i];
    }
    for(int i = 2 * half; i < hd; ++i) yr[i] = xr[i];
  }
}

// The keys row `end` attends from, 0-based: all, the last `win` (win > 0), or those of its own chunk of `-win` (win < 0).
static inline int lk_window_lo(int win, int end){ return win > 0 ? (end > win ? end - win : 0) : win < 0 ? (end - 1) / -win * -win : 0; }
// x soft-capped at c as the language writes it: c·(1 − 2/(e^(x·2/c) + 1)).
static inline float lk_cap(float v, float c){ return c > 0.f ? c * (1.f - 2.f / (lk_exp(v * (2.f / c)) + 1.f)) : v; }

// Attention of `rows` query rows of `heads` heads (row r's at q + r·qp) over a cache of `kvh` key/value heads (`heads /
// kvh` a group): row r attends positions [lo, end) of the cache — end = p0 + r + 1, or every one of the `l` where p0 < 0
// — keys `kp` and values `vp` floats a cache row apart; each score scaled, capped and softmaxed in f32 with a sink
// joining its head's denominator; row r's [heads][vd] at out + r·op; the cache `f32` or binary16 (`C`). A unit takes a
// row's key/value head with its group's query heads together over every key and value row it reads — the row loaded
// once for the group (`attn_gqa_row_packing`) — each head's sums in the order it takes alone.
template<typename C> struct lk_attn_job_t { const float* q; long qp; C kc; C vc; const float* sinks; float* out; long op; int rows, heads, kvh, hd, vd, p0, l, win; long kp, vp; float scale, cap; };
typedef lk_attn_job_t<const float*> lk_attn_job;
typedef lk_attn_job_t<const uint16_t*> lk_attn_job_h16;
typedef lk_attn_job_t<lk_i8rows> lk_attn_job_i8;
// The positions row r attends, [from, end).
template<typename C> static inline void lk_attention_span(const lk_attn_job_t<C>& j, int r, int& from, int& end){
  end = j.p0 >= 0 ? j.p0 + r + 1 : j.l;
  from = lk_window_lo(j.win, end);
}
// Row r's group g over positions [a, b): each head's scores, their exponentials below the largest and the values they
// weigh. Whole (a row's every position): the sink joins the largest and the denominator, and head hh's values summed by
// weight over the denominator land at o + hh·os. A split's partial: head hh's largest score, its denominator and its
// values summed by weight alone, at o + hh·os.
template<typename C> static void lk_attend_span(const lk_attn_job_t<C>& j, int r, int g, int a, int b, bool whole, float* o, long os){
  thread_local std::vector<float> w, m, l;
  const int group = j.heads / j.kvh, n = b - a, at = whole ? 0 : 2;
  const long stride = n + 8;
  m.resize(group); l.resize(group);
  w.resize((size_t)group * stride);
  const float* q0 = j.q + (long)r * j.qp + (long)g * group * j.hd;
  const bool sink = whole && j.sinks;
  for(int hh = 0; hh < group; ++hh) m[hh] = sink ? j.sinks[g * group + hh] : -INFINITY;
  for(int t = a; t < b; ++t){
    const auto kt = lk_cache_at(j.kc, (long)t * j.kp + (long)g * j.hd);
    for(int hh = 0; hh < group; ++hh){
      const float* qh = q0 + (long)hh * j.hd;
      __m256 acc = _mm256_setzero_ps();
      int i = 0;
      for(; i + 8 <= j.hd; i += 8) acc = _mm256_fmadd_ps(_mm256_loadu_ps(qh + i), lk_load8(kt + i), acc);
      float s = lk_hsum(acc);
      for(; i < j.hd; ++i) s += qh[i] * lk_load1(kt + i);
      s = lk_cap(s * j.scale, j.cap);
      w[hh * stride + t - a] = s;
      m[hh] = s > m[hh] ? s : m[hh];
    }
  }
  for(int hh = 0; hh < group; ++hh){
    float* wh = &w[hh * stride];
    float sum = sink ? lk_exp(j.sinks[g * group + hh] - m[hh]) : 0.f;
    int i = 0;
    __m256 lv = _mm256_setzero_ps();
    for(; i + 8 <= n; i += 8){ const __m256 e = lk_exp8(_mm256_sub_ps(_mm256_loadu_ps(wh + i), _mm256_set1_ps(m[hh]))); _mm256_storeu_ps(wh + i, e); lv = _mm256_add_ps(lv, e); }
    sum += lk_hsum(lv);
    for(; i < n; ++i){ wh[i] = lk_exp(wh[i] - m[hh]); sum += wh[i]; }
    l[hh] = sum;
    float* oh = o + hh * os;
    if(!whole){ oh[0] = m[hh]; oh[1] = sum; }
    for(int e = 0; e < j.vd; ++e) oh[at + e] = 0.f;
  }
  for(int t = a; t < b; ++t){
    const auto vt = lk_cache_at(j.vc, (long)t * j.vp + (long)g * j.vd);
    for(int hh = 0; hh < group; ++hh){
      float* oh = o + hh * os + at;
      const float wt = whole ? w[hh * stride + t - a] / l[hh] : w[hh * stride + t - a];
      const __m256 wv = _mm256_set1_ps(wt);
      int e = 0;
      for(; e + 8 <= j.vd; e += 8) _mm256_storeu_ps(oh + e, _mm256_fmadd_ps(wv, lk_load8(vt + e), _mm256_loadu_ps(oh + e)));
      for(; e < j.vd; ++e) oh[e] += wt * lk_load1(vt + e);
    }
  }
}
// A row attending more than `LK_ATTN_SPLIT` positions splits them (`attn_lse_merge`): each split's partial for every
// head of the group, then the partials merged by log-sum-exp with the sink's weight; a row within one split takes one
// pass. A row's bits are its span's — whatever rows run beside it, and whichever schedule runs it: its splits one after
// another in its unit, or spread over the threads (the parts, then the merge). Row r's group g keeps its partials at
// part + (r·kvh + g)·splits·group·(vd + 2): split s, head h at (s·group + h)·(vd + 2), its largest score, its
// denominator and its values.
constexpr int LK_ATTN_SPLIT = 256;
static inline int lk_attention_splits(int n){ return (n + LK_ATTN_SPLIT - 1) / LK_ATTN_SPLIT; }
template<typename C> static void lk_attention_merge_group(const lk_attn_job_t<C>& j, int r, int g, const float* part, int parts){
  thread_local std::vector<float> c;
  const int group = j.heads / j.kvh;
  const long step = (long)group * (j.vd + 2);
  c.resize(parts);
  for(int hh = 0; hh < group; ++hh){
    const float* p0 = part + (long)hh * (j.vd + 2);
    const float* sink = j.sinks ? j.sinks + g * group + hh : nullptr;
    float m = sink ? *sink : -INFINITY;
    for(int s = 0; s < parts; ++s) m = p0[s * step] > m ? p0[s * step] : m;
    float l = sink ? lk_exp(*sink - m) : 0.f;
    for(int s = 0; s < parts; ++s){ c[s] = lk_exp(p0[s * step] - m); l += p0[s * step + 1] * c[s]; }
    float* oh = j.out + (long)r * j.op + (long)(g * group + hh) * j.vd;
    for(int e = 0; e < j.vd; ++e){
      float acc = 0.f;
      for(int s = 0; s < parts; ++s) acc += p0[s * step + 2 + e] * c[s];
      oh[e] = acc / l;
    }
  }
}
// Split s of row r's group g: its partial at `part` (the group's), or row r's whole attention where it takes one pass.
template<typename C> static inline void lk_attention_split(const lk_attn_job_t<C>& j, int r, int g, int s, float* part){
  int from, end; lk_attention_span(j, r, from, end);
  const int group = j.heads / j.kvh;
  if(lk_attention_splits(end - from) <= 1){
    if(s == 0) lk_attend_span(j, r, g, from, end, true, j.out + (long)r * j.op + (long)g * group * j.vd, j.vd);
    return;
  }
  const int a = from + s * LK_ATTN_SPLIT;
  if(a < end) lk_attend_span(j, r, g, a, std::min(end, a + LK_ATTN_SPLIT), false, part + (long)s * group * (j.vd + 2), j.vd + 2);
}
// Units (row, key/value head), each running its row's splits and their merge.
template<typename C> static void lk_attention_units(const lk_attn_job_t<C>& j, long lo, long hi){
  thread_local std::vector<float> part;
  const int group = j.heads / j.kvh;
  for(long u = lo; u < hi; ++u){
    const int r = (int)(u / j.kvh), g = (int)(u % j.kvh);
    int from, end; lk_attention_span(j, r, from, end);
    const int parts = lk_attention_splits(end - from);
    if(parts <= 1){ lk_attention_split(j, r, g, 0, nullptr); continue; }
    part.resize((size_t)parts * group * (j.vd + 2));
    for(int s = 0; s < parts; ++s) lk_attention_split(j, r, g, s, part.data());
    lk_attention_merge_group(j, r, g, part.data(), parts);
  }
}
// The same attention spread over the threads: units (row, key/value head, split) of at most `splits` a row, then
// units (row, key/value head) merging each row's.
template<typename C> static void lk_attention_part_units(const lk_attn_job_t<C>& j, float* part, int splits, long lo, long hi){
  const int group = j.heads / j.kvh;
  for(long u = lo; u < hi; ++u){
    const long rg = u / splits;
    lk_attention_split(j, (int)(rg / j.kvh), (int)(rg % j.kvh), (int)(u % splits), part + rg * splits * group * (j.vd + 2));
  }
}
template<typename C> static void lk_attention_merge_units(const lk_attn_job_t<C>& j, const float* part, int splits, long lo, long hi){
  const int group = j.heads / j.kvh;
  for(long u = lo; u < hi; ++u){
    const int r = (int)(u / j.kvh), g = (int)(u % j.kvh);
    int from, end; lk_attention_span(j, r, from, end);
    const int parts = lk_attention_splits(end - from);
    if(parts > 1) lk_attention_merge_group(j, r, g, part + u * splits * group * (j.vd + 2), parts);
  }
}
// Unmasked attention of t rows of h heads over the same t rows' keys and values (an encoder's): units (row, head).
static void lk_full_attention_units(const float* q, const float* k, const float* v, float* out, int t, int h, int hd, long lo, long hi){
  thread_local std::vector<float> w;
  const long width = (long)h * hd;
  const float scale = 1.f / std::sqrt((float)hd);
  w.resize(t);
  for(long u = lo; u < hi; ++u){
    const int row = (int)(u / h), head = (int)(u % h);
    float m = -INFINITY;
    for(int jj = 0; jj < t; ++jj){
      float s = 0.f;
      for(int e = 0; e < hd; ++e) s += q[row * width + head * hd + e] * k[jj * width + head * hd + e];
      w[jj] = s * scale;
      m = w[jj] > m ? w[jj] : m;
    }
    float den = 0.f;
    for(int jj = 0; jj < t; ++jj){ w[jj] = lk_exp(w[jj] - m); den += w[jj]; }
    for(int e = 0; e < hd; ++e){
      float acc = 0.f;
      for(int jj = 0; jj < t; ++jj) acc += w[jj] / den * v[jj * width + head * hd + e];
      out[row * width + head * hd + e] = acc;
    }
  }
}

// Values [lo, hi) of the exp-made activations, each as the language writes it: silu x / (1 + e^−x); sigmoid
// 1 / (1 + e^−x); swiglu silu(g)·u; and one value's gelu x / (1 + e^(−2z)), z = √(2/π)·(x + 0.044715·x³).
static void lk_silu_vals(const float* x, float* y, long lo, long hi){
  long i = lo;
  for(; i + 8 <= hi; i += 8){ const __m256 v = _mm256_loadu_ps(x + i); _mm256_storeu_ps(y + i, _mm256_div_ps(v, _mm256_add_ps(_mm256_set1_ps(1.f), lk_exp8(_mm256_sub_ps(_mm256_setzero_ps(), v))))); }
  for(; i < hi; ++i) y[i] = x[i] / (1.f + lk_exp(0.f - x[i]));
}
static void lk_sigmoid_vals(const float* x, float* y, long lo, long hi){
  long i = lo;
  for(; i + 8 <= hi; i += 8){ const __m256 v = _mm256_loadu_ps(x + i); _mm256_storeu_ps(y + i, _mm256_div_ps(_mm256_set1_ps(1.f), _mm256_add_ps(_mm256_set1_ps(1.f), lk_exp8(_mm256_sub_ps(_mm256_setzero_ps(), v))))); }
  for(; i < hi; ++i) y[i] = 1.f / (1.f + lk_exp(0.f - x[i]));
}
static inline float lk_gelu1(float v){ const float z = 0.7978845608028654f * (v + 0.044715f * (v * (v * v))); return v / (1.f + lk_exp(0.f - 2.f * z)); }
// sign(v)·√max(|v|, eps).
static inline float lk_signed_root(float v, float eps){ return (v > 0.f ? 1.f : v < 0.f ? -1.f : 0.f) * std::sqrt(std::max(std::fabs(v), eps)); }
static void lk_swiglu_vals(const float* g, const float* u, float* y, long lo, long hi){
  long i = lo;
  for(; i + 8 <= hi; i += 8){
    const __m256 gv = _mm256_loadu_ps(g + i);
    _mm256_storeu_ps(y + i, _mm256_mul_ps(_mm256_div_ps(gv, _mm256_add_ps(_mm256_set1_ps(1.f), lk_exp8(_mm256_sub_ps(_mm256_setzero_ps(), gv)))), _mm256_loadu_ps(u + i)));
  }
  for(; i < hi; ++i) y[i] = g[i] / (1.f + lk_exp(0.f - g[i])) * u[i];
}

// Channels [lo, hi) of a causal convolution over `rows` rows: each row shifts the channel's `taps` cached inputs one
// back and takes its own last, then sums them against the channel's weights.
static void lk_causal_conv_units(const float* x, long xp, const float* wc, float* cs, float* y, int taps, int chan, int rows, long lo, long hi){
  for(long ch = lo; ch < hi; ++ch)
    for(int r = 0; r < rows; ++r){
      for(int t = 1; t < taps; ++t) cs[(long)(t - 1) * chan + ch] = cs[(long)t * chan + ch];
      cs[(long)(taps - 1) * chan + ch] = x[(long)r * xp + ch];
      float acc = 0.f;
      for(int t = 0; t < taps; ++t) acc += cs[(long)t * chan + ch] * wc[(long)t * chan + ch];
      y[(long)r * chan + ch] = acc;
    }
}

// Heads [lo, hi) of a gated delta rule over `rows` rows: each row's head reads its state's prediction of its key, moves
// the state toward its value by β after decaying it by α, reads the state at its query scaled by 1/√d, and writes that
// read normed (an RMS norm by `gn`) times its gate of z. Value head `h` takes key head `h mod hk`.
struct lk_gdn_job { const float* q; const float* k; const float* v; const float* z; const float* b; const float* a; float* st; const float* gn; float* out; int hv, hk, d, rows; float eps; int sigmoid_gate; };
static void lk_gated_delta_units(const lk_gdn_job& j, long lo, long hi){
  thread_local std::vector<float> u, o;
  const int d = j.d;
  u.resize(d); o.resize(d);
  const float read_scale = 1.f / std::sqrt((float)d);
  for(long head = lo; head < hi; ++head)
    for(int r = 0; r < j.rows; ++r){
      const long rq = (long)r * j.hk * d, rv = (long)r * j.hv * d, rh = (long)r * j.hv;
      const long kb = rq + (head % j.hk) * d, vb = rv + head * d, sb = head * (long)d * d;
      const float alpha = j.a[rh + head], beta = j.b[rh + head];
      for(int x = 0; x < d; ++x){ float acc = 0.f; for(int y = 0; y < d; ++y) acc += j.st[sb + (long)x * d + y] * j.k[kb + y]; u[x] = acc; }
      for(int x = 0; x < d; ++x){
        const float coef = beta * (j.v[vb + x] - alpha * u[x]);
        for(int y = 0; y < d; ++y) j.st[sb + (long)x * d + y] = alpha * j.st[sb + (long)x * d + y] + coef * j.k[kb + y];
      }
      float ssq = 0.f;
      for(int x = 0; x < d; ++x){ float acc = 0.f; for(int y = 0; y < d; ++y) acc += j.st[sb + (long)x * d + y] * j.q[kb + y]; o[x] = acc * read_scale; ssq += o[x] * o[x]; }
      const float den = std::sqrt(ssq / d + j.eps);
      for(int x = 0; x < d; ++x){
        const float zv = j.z[vb + x], gate = j.sigmoid_gate ? 1.f / (1.f + lk_exp(0.f - zv)) : zv / (1.f + lk_exp(0.f - zv));
        j.out[vb + x] = j.gn[x] * (o[x] / den) * gate;
      }
    }
}

// A mixture of experts for one row, in phases: the router's logits (units: experts), the selection (one unit), each
// kept expert's gate and up rows (units: kept × dff), the hidden rows quantized (blocks), and the down rows summed over
// the kept experts in order (units: d). The router reads the activation's values under either numerics — which experts
// a row takes is not the codes' to decide — and the experts' products read it one word a block where `one_word`. A matrix is checkpoint rows of format `f` (8 Q8_0, 12 Q4_K, 14 Q6_K, 0 f32) or
// values (`vals`: the router d × ne, the experts by output).
struct lk_moe_mat { int f; const uint8_t* rows; long pitch; const float* vals; };
struct lk_moe_job {
  const float* x; lk_moe_mat wr, wg, wu, wd; const float *rb, *bg, *bu, *bd, *bs;
  float* out; int d, dff, ne, k; int renormalize, sigmoid_score, weigh_inputs, groups, kept, clamped; float limit, slope, scale; int one_word;
  int8_t* xq; float* xd; int* xs; float* logits; int* chosen; float* weights; float* gate; float* up; float* hidden; int8_t* hq; float* hd; int* hs;
};
// Outputs [o, o + n) of rows of checkpoint matrix `m` (each `k` values) against an activation, one word a block (`q`,
// `d`, `s`) or its values (`x`), into y: one product over the run, each output the bits it is alone.
static void lk_moe_dots(const lk_moe_mat& m, long o, long n, int k, const float* x, const int8_t* q, const float* d, const int* s, float* y){
  if(m.f == 8 && q){ lk_q8_job j{m.rows + o * m.pitch, m.pitch, q, d, y, 1, (int)n, k, n}; lk_gemm_q8_0_rows(j, 0, n); }
  else if(m.f == 8){ lk_q8f_job j{m.rows + o * m.pitch, m.pitch, x, k, y, 1, (int)n, k, n}; lk_gemm_q8_0_f32_units(j, 0, n); }
  else if(m.f == 12 || m.f == 14){ lk_kq_job j{m.f, m.rows + o * m.pitch, m.pitch, q, d, s, x, k, y, 1, (int)n, k, n}; lk_gemm_kq_units(j, 0, n); }
  else { const float* w = m.f == 0 && m.rows ? (const float*)m.rows + o * k : m.vals + o * k; lk_f32_job j{w, k, x, k, y, 1, (int)n, k, n}; lk_gemm_f32_units(j, 0, n); }
}
static void lk_moe_router(const lk_moe_job& j, long lo, long hi){
  if(j.wr.vals)
    for(long e = lo; e < hi; ++e){ float acc = 0.f; for(int t = 0; t < j.d; ++t) acc += j.x[t] * j.wr.vals[(long)t * j.ne + e]; j.logits[e] = acc; }
  else lk_moe_dots(j.wr, lo, hi - lo, j.d, j.x, nullptr, nullptr, nullptr, j.logits + lo);
  if(j.rb) for(long e = lo; e < hi; ++e) j.logits[e] = j.logits[e] + j.rb[e];
}
static void lk_moe_select(const lk_moe_job& j){
  const int ne = j.ne;
  std::vector<float> p(ne), pm(ne);
  if(j.sigmoid_score) for(int e = 0; e < ne; ++e) p[e] = 1.f / (1.f + lk_exp(0.f - j.logits[e]));
  else {
    float mx = j.logits[0];
    for(int e = 1; e < ne; ++e) mx = j.logits[e] > mx ? j.logits[e] : mx;
    float sum = 0.f;
    for(int e = 0; e < ne; ++e) sum += lk_exp(j.logits[e] - mx);
    for(int e = 0; e < ne; ++e) p[e] = lk_exp(j.logits[e] - mx) / sum;
  }
  for(int e = 0; e < ne; ++e) pm[e] = j.bs ? p[e] + j.bs[e] : p[e];
  if(j.groups > 0){
    const int per = ne / j.groups;
    std::vector<float> score(j.groups), kept(ne, -1.0e30f);
    for(int g = 0; g < j.groups; ++g){
      float t1 = -1.0e30f, t2 = -1.0e30f;
      for(int e = g * per; e < (g + 1) * per; ++e){ if(pm[e] > t1){ t2 = t1; t1 = pm[e]; } else if(pm[e] > t2) t2 = pm[e]; }
      score[g] = per > 1 ? t1 + t2 : t1;
    }
    for(int c = 0; c < j.kept; ++c){
      float bv = -1.0e30f; int bi = 0;
      for(int g = 0; g < j.groups; ++g) if(score[g] > bv){ bv = score[g]; bi = g; }
      for(int e = bi * per; e < (bi + 1) * per; ++e) kept[e] = pm[e];
      score[bi] = -2.0e30f;
    }
    pm = kept;
  }
  float wsum = 0.f;
  for(int c = 0; c < j.k; ++c){
    float bv = -1.0e30f; int bi = 0;
    for(int e = 0; e < ne; ++e) if(pm[e] > bv){ bv = pm[e]; bi = e; }
    wsum += p[bi]; j.chosen[c] = bi; j.weights[c] = p[bi]; pm[bi] = -1.0e30f;
  }
  if(j.renormalize){ const float s = wsum > 6.103515625e-5f ? wsum : 6.103515625e-5f; for(int c = 0; c < j.k; ++c) j.weights[c] /= s; }
  for(int c = 0; c < j.k; ++c) j.weights[c] *= j.scale;
}
static void lk_moe_gate_up(const lk_moe_job& j, long lo, long hi){
  thread_local std::vector<float> gs, us;
  for(long u0 = lo; u0 < hi;){
    const int c = (int)(u0 / j.dff), e = j.chosen[c];
    const long u1 = std::min(hi, (long)(c + 1) * j.dff), r0 = (long)e * j.dff + u0 % j.dff;
    gs.resize(u1 - u0); us.resize(u1 - u0);
    lk_moe_dots(j.wg, r0, u1 - u0, j.d, j.x, j.one_word && j.wg.f != 0 ? j.xq : nullptr, j.xd, j.xs, gs.data());
    lk_moe_dots(j.wu, r0, u1 - u0, j.d, j.x, j.one_word && j.wu.f != 0 ? j.xq : nullptr, j.xd, j.xs, us.data());
    for(long u = u0; u < u1; ++u){
      const long row = r0 + (u - u0);
      float g = gs[u - u0], up = us[u - u0];
      if(j.weigh_inputs){ g = j.weights[c] * g; up = j.weights[c] * up; }
      if(j.bg) g += j.bg[row];
      if(j.bu) up += j.bu[row];
      float h;
      if(j.clamped){
        const float gc = g < j.limit ? g : j.limit, uc = up < j.limit ? (up > -j.limit ? up : -j.limit) : j.limit;
        h = gc / (1.f + lk_exp(0.f - j.slope * gc)) * (uc + 1.f);
      } else h = g / (1.f + lk_exp(0.f - g)) * up;
      j.hidden[u] = h;
    }
    u0 = u1;
  }
}
static void lk_moe_down(const lk_moe_job& j, long lo, long hi){
  thread_local std::vector<float> ys, acc;
  ys.resize(hi - lo); acc.assign(hi - lo, 0.f);
  for(int c = 0; c < j.k; ++c){
    const int e = j.chosen[c];
    lk_moe_dots(j.wd, (long)e * j.d + lo, hi - lo, j.dff, j.hidden + (long)c * j.dff, j.one_word && j.wd.f != 0 ? j.hq + (long)c * j.dff : nullptr, j.hd + (long)c * (j.dff / 32), j.hs + (long)c * (j.dff / 32), ys.data());
    for(long oo = lo; oo < hi; ++oo){
      float y = ys[oo - lo];
      if(j.bd) y += j.bd[(long)e * j.d + oo];
      acc[oo - lo] += (j.weigh_inputs ? 1.f : j.weights[c]) * y;
    }
  }
  for(long oo = lo; oo < hi; ++oo) j.out[oo] = acc[oo - lo];
}

// Per-head products, each output (t, h, r) of `m` rows by `heads` heads: y[t][h][r] = Σ_e x[t][h][e] · W[h·a + e][r], the
// activation as its values. W is checkpoint rows of format `f` (8 Q8_0, 12 Q4_K, 14 Q6_K, 0 f32) — by output
// (`transposed`: row h·b + r holds head h's a values for output r) or by input (row h·a + e holds its b outputs' values)
// — or values laid out as W, `vals`. Units: outputs in y's order.
struct lk_hm_job { const float* x; long xp; int f; const uint8_t* rows; long pitch; const float* vals; int transposed; float* y; long yp; int m, heads, a, b; };
static void lk_deq_row(int f, const uint8_t* rows, long pitch, long row, int cols, float* y);
static void lk_head_matmul_units(const lk_hm_job& j, long lo, long hi){
  thread_local std::vector<float> w;
  long held = -1;
  for(long u = lo; u < hi; ++u){
    const long t = u / ((long)j.heads * j.b), h = (u / j.b) % j.heads, r = u % j.b;
    const float* xh = j.x + t * j.xp + h * j.a;
    float acc = 0.f;
    if(j.vals) for(int e = 0; e < j.a; ++e) acc += xh[e] * j.vals[((long)h * j.a + e) * j.b + r];
    else if(j.transposed && j.f == 8 && j.a % 32 == 0){ lk_q8f_job q{j.rows + (h * j.b + r) * j.pitch, j.pitch, xh, j.a, &acc, 1, 1, j.a, 1}; lk_gemm_q8_0_f32_units(q, 0, 1); }
    else if(j.transposed){
      w.resize(j.a); lk_deq_row(j.f, j.rows, j.pitch, h * j.b + r, j.a, w.data());
      for(int e = 0; e < j.a; ++e) acc += xh[e] * w[e];
    } else {
      if(held != h){ w.resize((size_t)j.a * j.b); for(int e = 0; e < j.a; ++e) lk_deq_row(j.f, j.rows, j.pitch, h * j.a + e, j.b, w.data() + (size_t)e * j.b); held = h; }
      for(int e = 0; e < j.a; ++e) acc += xh[e] * w[(size_t)e * j.b + r];
    }
    j.y[t * j.yp + h * j.b + r] = acc;
  }
}

// Row `idx` of a table of Q8_0 or K-quant rows read in place, dequantized, times `scale`.
static void lk_gather_q8_0(const uint8_t* t, long pitch, long idx, float* y, int cols, float scale){
  const uint8_t* row = t + idx * pitch;
  for(int b = 0; b < cols / 32; ++b){
    uint16_t dh; std::memcpy(&dh, row + 34 * b, 2);
    const float d = lk_f16(dh) * scale;
    const int8_t* q = (const int8_t*)(row + 34 * b + 2);
    for(int i = 0; i < 32; ++i) y[32 * b + i] = d * q[i];
  }
}
static void lk_gather_kq(int f, const uint8_t* t, long pitch, long idx, float* y, int cols, float scale){
  const uint8_t* row = t + idx * pitch;
  for(int b = 0; b < cols / 256; ++b){
    if(f == 12) lk_deq_q4_k(row + 144L * b, y + 256 * b); else lk_deq_q6_k(row + 210L * b, y + 256 * b);
    if(scale != 1.f) for(int i = 0; i < 256; ++i) y[256 * b + i] *= scale;
  }
}

// Row `row` of checkpoint rows of format `f` (8 Q8_0, 12 Q4_K, 14 Q6_K, 0 f32) dequantized into `y`.
static void lk_deq_row(int f, const uint8_t* rows, long pitch, long row, int cols, float* y){
  if(f == 8) lk_gather_q8_0(rows, pitch, row, y, cols, 1.f);
  else if(f == 12 || f == 14) lk_gather_kq(f, rows, pitch, row, y, cols, 1.f);
  else std::memcpy(y, rows + row * pitch, sizeof(float) * cols);
}

// The largest of values [lo, hi), the first of equals: its value and index into `best` (one pair a chunk of `grain`).
static void lk_argmax_part(const float* x, long lo, long hi, float* bv, long* bi){ long b = lo; for(long i = lo + 1; i < hi; ++i) if(x[i] > x[b]) b = i; *bv = x[b]; *bi = b; }
"#;
