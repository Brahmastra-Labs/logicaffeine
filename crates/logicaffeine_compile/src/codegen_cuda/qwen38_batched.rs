//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! ★ BATCHED Qwen3.8-27B DECODE ★ — where the bandwidth wall stops being the wall.
//!
//! Batch-1 decode reads all 27.3 GB of weights to produce ONE token. That is the entire reason it is
//! bandwidth-bound, and the reason a 4% throughput difference is a hard-won result there. At batch B the same
//! weight pass serves B sequences, so the per-token weight traffic falls by B and aggregate throughput scales
//! close to linearly until the arithmetic takes over.
//!
//! The whole trick lives in one kernel: load each weight `int4` ONCE into registers and reuse it across the
//! sequences in a batch tile, instead of re-reading the entire matrix per sequence. Everything else here is
//! the single-sequence program with a sequence index threaded through it — the recurrence state, the KV
//! cache and the conv ring become per-sequence, and the norms become one block per sequence.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

use super::qwen38::Qwen38Cfg;

/// Sequences whose accumulators a single warp carries in registers. Above this the register file, not the
/// bandwidth, decides the shape — so a larger batch is tiled over `grid.y` rather than over registers.
pub const BTG: usize = 8;

/// The batched device code. Every kernel is the single-sequence form with a sequence index added; the GEMV is
/// the one that changes shape, because it is the one that decides the token time.
pub const QWEN38_BATCHED_KERNELS: &str = r#"
// `BMAX` bounds the per-thread accumulator array in the staged GEMVs. The emitted program and the
// equivalence tests both define it ahead of this text, but the kernel set must still compile when a
// caller does not — a shipped kernel string that fails on a missing macro is a trap for the next
// harness that includes it. The guard is inert whenever a real batch bound was supplied.
// `BTG` (sequences per warp-tile) and `MAXSEQ` (KV-cache depth) were referenced here without
// defaults long before the MMQ work — the self-containment test found them the moment it existed.
// Same class, same fix.
#ifndef BTG
#define BTG 8
#endif
#ifndef MAXSEQ
#define MAXSEQ 64
#endif
#ifndef BMAX
#define BMAX 32
#endif
// The MMQ tile's shape constants. Same rule as BMAX and for the same reason: the emitted program and
// the equivalence tests define these ahead of this text, but the kernel set must still compile when a
// caller does not. Adding them WITHOUT guards broke nine tests an hour after BMAX was fixed — the
// identical failure, reintroduced. `the_kernel_set_compiles_with_no_defines_supplied` is the standing
// check that stops a third instance.
#ifndef MMQ_TJ
#define MMQ_TJ 32
#endif
#ifndef MMQ_RI
#define MMQ_RI 2
#endif
#ifndef MMQ_RJ
#define MMQ_RJ 2
#endif
#ifndef MMQ_KC
#define MMQ_KC 256
#endif
#ifndef MMQ_ACC
#define MMQ_ACC double
#endif
// The tensor-core tile's shape constants, same guard rule and same reason as the MMQ set above.
// TC_TJ must be 8, 16, 32 or 64: the warp -> sub-tile map needs TC_TJ/8 to divide the 8 warps.
#ifndef TC_TJ
#define TC_TJ 32
#endif
// How many 256-thread tile blocks ptxas must keep resident per SM. MEASURED on an H100 NVL
// (2026-09-20): the tile compiles to 102 registers, so 256 threads x 102 = 26,112 of the SM's 65,536
// and only TWO blocks fit -- 25% occupancy. nsys puts the tile at 70.9% of the step while it uses
// 9.9% of tensor-core peak and 24% of achievable bandwidth: it is neither math- nor bandwidth-bound,
// it is OCCUPANCY-bound, which is why halving its arithmetic bought only 6%.
// 1 lets ptxas take every register it wants — the behaviour this replaces.
// The analogous cap was worth +18.9% on the 30B MoE trio, where the optimum was an INTERIOR point
// (mb=5 best, mb=6 spilled), so this is a knob to sweep and never a constant to guess.
#ifndef TC_DYNSMEM
#define TC_DYNSMEM 0
#endif
// ── IN-KERNEL STALL ATTRIBUTION ──────────────────────────────────────────────────────────────
// `ncu` is permission-blocked in RunPod containers, which is why every attribution in this campaign
// came from ablation and roofline arithmetic. `clock64()` needs no privileges and answers the one
// question those cannot: where does a warp actually WAIT? One lane per warp accumulates, so the
// counters cost a handful of atomics per chunk and the shipped path compiles them away entirely.
#ifndef TC_PROBE
#define TC_PROBE 0
#endif
#ifndef TC_TILED_W
#define TC_TILED_W 0
#endif
#if TC_PROBE
__device__ unsigned long long bq_tc_clk[4];
#define BQ_CLK(v) unsigned long long v = clock64();
#define BQ_ACC(i,a,b) if((threadIdx.x&31)==0) atomicAdd(&bq_tc_clk[i],(unsigned long long)((b)-(a)));
#else
#define BQ_CLK(v)
#define BQ_ACC(i,a,b)
#endif
// ★ THE FALLBACK MUST BE THE SHIPPED VALUE. It used to be 1, which is a residency the program never
// asks for: the emitter always writes `tc_occ_def` (3 on Hopper, 2 below) ahead of this guard. Anything
// that compiles these kernels WITHOUT the emitter -- the decode bandwidth bench, most obviously -- then
// silently measured one block per SM. That is 167 GB/s against the shipped 199 at the decode shape on a
// 3060: the instrument was slandering the tile by 19%, which is the same defect as the bench that once
// ran SPLIT=1. A default that no shipped configuration uses is a trap, not a default.
#ifndef TC_MINBLOCKS
#if defined(__CUDA_ARCH__) && __CUDA_ARCH__ >= 900
#define TC_MINBLOCKS 3
#else
#define TC_MINBLOCKS 2
#endif
#endif
// How many K-chunks the tile keeps staged at once. 1 is the shipped, measured behaviour: a ONE-deep
// prefetch held in REGISTERS (`pw`), which costs ~32 registers at B=8 and is why the tile compiles to
// 181 there (one block per SM). A deeper pipeline has to live in shared memory instead, which is what
// `cp.async` and Hopper's TMA both exist to fill -- and the measured need is large: the tile moves
// 24.33 GB at 728 GB/s while `bq_gdn_step` moves 9.6 GB at 2.0 TB/s in the same step on the same chip.
// This dimension is the structural prerequisite for either. At TC_STAGES=1 every stage index folds to
// 0 and the emitted code is the one that produced every number on record.
#ifndef TC_STAGES
#define TC_STAGES 1
#endif
#ifndef TC_WM
#define TC_WM 4
#endif
#ifndef TC_KC
#define TC_KC 128
#endif
// The row-parallel GEMV's shape. MV_TJ sequences live in registers, MV_RB output rows share each
// activation load, and MV_NW warps cooperate on one row-group's K reduction. Same guard rule as above.
#ifndef MV_TJ
#define MV_TJ 8
#endif
#ifndef MV_RB
#define MV_RB 2
#endif
#ifndef MV_NW
#define MV_NW 1
#endif
// The residual-norm-quantise kernels reduce over the whole model dimension, so they take ONE BLOCK
// PER SEQUENCE — at two sequences that is two blocks of a 132-SM part, and the block width is the
// only knob left that buys memory-level parallelism. It was a literal 256; measured on an A100 at
// B=2, widening to 1024 took the step from 31.05 ms to 29.51 ms (+5%) for no other change. Sizing a
// block by a constant when its grid cannot fill the machine is the bug; the constant is named here
// so the next person sees the choice instead of the number.
#ifndef RMSN_NT
#define RMSN_NT 1024
#endif
// How many chunks the split residual norm cuts the model dimension into. Only read on the fast lane.
#ifndef NORM_G
#define NORM_G 20
#endif

// ══ THE BATCHED GEMV, STAGED ════════════════════════════════════════════════════════════════════
// Corrected target. llama.cpp does NOT switch to tensor cores at these batch sizes — mmq.cu:327 reads
// `!fp16_mma_hardware_available(cc) || ne11 < MMQ_DP4A_MAX_BATCH_SIZE` with that constant set to 64, so for
// every batch below 64 it runs a DP4A kernel. Its advantage over ours was never the instruction; it was the
// STAGING. Our register-tiled form re-reads each sequence's activation slab from L2 for every output row,
// which at B=32 is ~16x more activation traffic than weight traffic and is what pinned us at 128 GB/s of an
// available 3000.
//
// Here the activation tile for ALL sequences is staged once per 512-wide K block in shared memory, and every
// warp in the block reads it from there. Weights still stream from global — they are touched once and never
// reused, so staging them would only add a copy.
//
// Structure follows llama.cpp's MMQ (ggml/src/ggml-cuda/mmq.cuh, MIT, (c) 2023-2024 The ggml authors);
// the code is ours. Crucially the ACCUMULATION ORDER is unchanged from `bq_gemv_h` — same ascending K
// blocks, same per-block scale product — so this is bit-identical, not merely close.
__global__ void bq_gemv_h_smem(float* __restrict__ Y, const signed char* __restrict__ wq,
                               const __half* __restrict__ ws, const signed char* __restrict__ Xq,
                               const float* __restrict__ Xs, int K, int N, int Bn, int YS){
  int warps = blockDim.x >> 5;
  int o = blockIdx.x * warps + (threadIdx.x >> 5);
  int lane = threadIdx.x & 31, tid = threadIdx.x;
  int KB = K / 32;
  extern __shared__ char gsmem[];
  signed char* Asm = (signed char*)gsmem;                  // Bn x 512 staged activations
  float*       Ass = (float*)(gsmem + Bn * 512);           // Bn x 16 staged scales
  float acc[BMAX];
  #pragma unroll
  for(int t=0;t<BMAX;t++) acc[t]=0.f;
  const signed char* wrow = wq + (long)(o < N ? o : 0) * K;
  const __half*      wsr  = ws + (long)(o < N ? o : 0) * KB;
  for(int k0=0;k0<K;k0+=512){
    for(int i=tid;i<Bn*512;i+=blockDim.x){ int b=i>>9, c=i&511;
      Asm[i] = (k0+c<K) ? Xq[(long)b*K+k0+c] : (signed char)0; }
    for(int i=tid;i<Bn*16;i+=blockDim.x){ int b=i>>4, c=i&15;
      Ass[i] = (k0/32+c<KB) ? Xs[(long)b*KB+k0/32+c] : 0.f; }
    __syncthreads();
    if(o < N && k0+lane*16 < K){
      int4 w = *(const int4*)(wrow + k0 + lane*16);
      int blk = lane >> 1;                                  // which 32-block of this 512 the lane covers
      float wsb = __half2float(wsr[k0/32 + blk]);
      #pragma unroll
      for(int t=0;t<BMAX;t++){
        if(t>=Bn) break;
        int4 xx = *(const int4*)(Asm + t*512 + lane*16);
        int d = __dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
        acc[t] += (float)d * (wsb * Ass[t*16 + blk]);
      }
    }
    __syncthreads();
  }
  #pragma unroll
  for(int t=0;t<BMAX;t++){
    if(t>=Bn) break;
    float a=acc[t];
    for(int r=16;r>0;r>>=1) a+=__shfl_down_sync(0xffffffffu,a,r);
    if(lane==0 && o<N) Y[(long)t*YS+o]=a;
  }
}


// The TWO-PASS activation form, staged. This is the lane the accuracy result rests on — `bq_gemv_hl`
// is what runs when the emitter is asked for the golden/oracle-closest decode — and it was the one
// lane the staging fix did not cover, so it kept paying the L2 re-read that pinned the batch at
// 128 GB/s of an available 3000.
//
// Both activation words are staged, so the shared-memory footprint is 2*Bn*512 + Bn*64 bytes
// (34 KB at Bn=32, inside the 48 KB default block limit). Weights still stream from global: each
// weight byte is read once per output row and never reused, so staging them would only add a copy.
//
// ACCUMULATION ORDER IS UNCHANGED from `bq_gemv_hl`: the same ascending 512-wide K chunks, the same
// `blk = k0/32 + lane/2` block-scale index, and the same
// `((float)dh + (float)dl/127) * (wsb * xs)` grouping per chunk. That is what makes this a free
// substitution rather than a numerically different kernel — the golden token stream cannot move.
// Every K in this model is a multiple of 512 (5120, 17408, 2048, 6144, 1024, 248320), so the
// zero-padded staging covers the whole reduction and `bq_gemv_hl`'s scalar tail loop is empty.
__global__ void bq_gemv_hl_smem(float* __restrict__ Y, const signed char* __restrict__ wq,
                                const __half* __restrict__ ws, const signed char* __restrict__ Xh,
                                const signed char* __restrict__ Xl, const float* __restrict__ Xs,
                                int K, int N, int Bn, int YS){
  int warps = blockDim.x >> 5;
  int o = blockIdx.x * warps + (threadIdx.x >> 5);
  int lane = threadIdx.x & 31, tid = threadIdx.x;
  int KB = K / 32;
  extern __shared__ char gsmem[];
  signed char* Hsm = (signed char*)gsmem;                        // Bn x 512 staged hi words
  signed char* Lsm = (signed char*)(gsmem + Bn * 512);           // Bn x 512 staged lo words
  float*       Ass = (float*)(gsmem + 2 * Bn * 512);             // Bn x 16 staged block scales
  float acc[BMAX];
  #pragma unroll
  for(int t=0;t<BMAX;t++) acc[t]=0.f;
  const signed char* wrow = wq + (long)(o < N ? o : 0) * K;
  const __half*      wsr  = ws + (long)(o < N ? o : 0) * KB;
  for(int k0=0;k0<K;k0+=512){
    for(int i=tid;i<Bn*512;i+=blockDim.x){ int b=i>>9, c=i&511;
      int src = k0+c;
      Hsm[i] = (src<K) ? Xh[(long)b*K+src] : (signed char)0;
      Lsm[i] = (src<K) ? Xl[(long)b*K+src] : (signed char)0; }
    for(int i=tid;i<Bn*16;i+=blockDim.x){ int b=i>>4, c=i&15;
      Ass[i] = (k0/32+c<KB) ? Xs[(long)b*KB+k0/32+c] : 0.f; }
    __syncthreads();
    if(o < N && k0+lane*16 < K){
      int4 w = *(const int4*)(wrow + k0 + lane*16);
      int blk = lane >> 1;
      float wsb = __half2float(wsr[k0/32 + blk]);
      #pragma unroll
      for(int t=0;t<BMAX;t++){
        if(t>=Bn) break;
        int4 hh = *(const int4*)(Hsm + t*512 + lane*16);
        int4 ll = *(const int4*)(Lsm + t*512 + lane*16);
        int dh = __dp4a(w.x,hh.x,0)+__dp4a(w.y,hh.y,0)+__dp4a(w.z,hh.z,0)+__dp4a(w.w,hh.w,0);
        int dl = __dp4a(w.x,ll.x,0)+__dp4a(w.y,ll.y,0)+__dp4a(w.z,ll.z,0)+__dp4a(w.w,ll.w,0);
        acc[t] += ((float)dh + (float)dl*(1.0f/127.0f)) * (wsb * Ass[t*16 + blk]);
      }
    }
    __syncthreads();
  }
  #pragma unroll
  for(int t=0;t<BMAX;t++){
    if(t>=Bn) break;
    float a=acc[t];
    for(int r=16;r>0;r>>=1) a+=__shfl_down_sync(0xffffffffu,a,r);
    if(lane==0 && o<N) Y[(long)t*YS+o]=a;
  }
}


// ══ THE MMQ-SHAPED TILE — the lowering that actually wins at batch ══════════════════════════════
//
// MEASURED, H100 NVL, isolated projection N=17408 K=5120, double accumulators, KC=256:
//   B=16  0.185 -> 0.194 ms  (0.95x, LOSS)
//   B=32  0.347 -> 0.285 ms  (1.22x)
//   B=64  0.673 -> 0.484 ms  (1.39x)
// The win grows with the batch, so this is a shape rule with a threshold, not a replacement.
//
// WHY the register-tiled form runs out of road, which is the whole reason this kernel exists. It
// reuses each WEIGHT element across all B sequences but each ACTIVATION element across exactly one
// output row, so at B=32 it issues 64 activation loads per weight load. Neither escape works inside a
// warp-register tile: BTG must cover the batch or the weight matrix is re-streamed (4x at B=32), and
// rows-per-warp needs RI*BTG accumulators — registers measured 40 -> 63 -> 96 -> 168 as RI went 1 to
// 8. Shared memory breaks the coupling: the tile size stops being a register-count problem, and each
// thread keeps only its own RI x RJ accumulators.
//
// Two properties are load-bearing and were both learned the expensive way:
//   * The thread -> output map must be a BIJECTION onto the tile. With (TI/RI)*(TJ/RJ) < NT the
//     surplus threads wrap and recompute outputs another thread already produced: silently correct,
//     4x the work. The static_assert below is that bug, caught.
//   * The shared rows must be PADDED. Unpadded rows are KC bytes apart and shared memory repeats
//     every 128 B, so every thread of a warp — which differ by row — hits the same bank: a 32-way
//     conflict on every read of the compute loop. One int of padding took the kernel from 0.704 ms
//     to 0.450 ms on the development card. It is the difference between losing and winning.
//
// Accumulation is in double: the per-32-block int32 dot is exact, so the only rounding is the sum of
// K/32 float terms, and fp64 is half-rate on a datacentre part (measured free here: 1.22x with double
// against 1.25x with float) while being 10x more accurate than the float sum. On consumer cards fp64
// is 1/32 rate and this choice would be wrong — which is why it is a knob, not a constant.
//
// Tile shape and K-step follow llama.cpp's MMQ (ggml/src/ggml-cuda/mmq.cuh, MIT, (c) 2023-2024 The
// ggml authors): 8 warps, 256 K-elements staged per barrier round. A sweep of KC over 128/256/512
// independently picked 256, which is their MMQ_ITER_K. The DESIGN PARAMETERS are theirs; the code is
// ours. Attribution recorded in benchmarks/results/qwen38_27b.json.
//
// This re-associates the K reduction, so unlike the staged GEMVs it is NOT bit-identical to the
// register-tiled form; it is held to a tolerance by `batched_equivalence_lock`, exactly as the
// tensor-core lowering is, and the golden token stream is the acceptance test.
__global__ void __launch_bounds__(256) bq_gemm_mmq(
    float* __restrict__ Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh, const signed char* __restrict__ Xl,
    const float* __restrict__ Xs, int K, int N, int B, int YS){
  constexpr int NCG = MMQ_TJ / MMQ_RJ;      // column groups fixed by the sequence tile
  constexpr int NRG = 256 / NCG;            // row groups take the rest of the block
  constexpr int TI  = NRG * MMQ_RI;         // ...which DERIVES the output-row tile
  constexpr int SW  = MMQ_KC/4 + 1;         // ints per shared row; +1 puts rows on consecutive banks
  static_assert(MMQ_TJ % MMQ_RJ == 0 && NCG * NRG == 256,
                "thread -> output map must be a bijection onto the tile");
  __shared__ int   sW[TI][SW];
  __shared__ int   sH[MMQ_TJ][SW];
  __shared__ int   sL[MMQ_TJ][SW];
  __shared__ __half sWs[TI][MMQ_KC/32];
  __shared__ float  sXs[MMQ_TJ][MMQ_KC/32];

  const int tid = threadIdx.x;
  const int i0  = blockIdx.x * TI;
  const int KB  = K/32;
  const int rg = tid % NRG, cg = tid / NRG;
  const int ir0 = rg*MMQ_RI, jr0 = cg*MMQ_RJ;

  MMQ_ACC acc[MMQ_RI][MMQ_RJ];
  #pragma unroll
  for(int a=0;a<MMQ_RI;a++)
    #pragma unroll
    for(int b=0;b<MMQ_RJ;b++) acc[a][b]=(MMQ_ACC)0;

  for(int k0=0;k0<K;k0+=MMQ_KC){
    for(int idx=tid; idx<TI*(MMQ_KC/16); idx+=256){
      int r=idx/(MMQ_KC/16), c=idx%(MMQ_KC/16); int o=i0+r;
      const int4 v = (o<N && k0+c*16<K) ? ((const int4*)(wq+(long)o*K+k0))[c] : make_int4(0,0,0,0);
      sW[r][c*4+0]=v.x; sW[r][c*4+1]=v.y; sW[r][c*4+2]=v.z; sW[r][c*4+3]=v.w;
    }
    for(int idx=tid; idx<MMQ_TJ*(MMQ_KC/16); idx+=256){
      int r=idx/(MMQ_KC/16), c=idx%(MMQ_KC/16);
      const int4 h = (r<B && k0+c*16<K) ? ((const int4*)(Xh+(long)r*K+k0))[c] : make_int4(0,0,0,0);
      const int4 l = (r<B && k0+c*16<K) ? ((const int4*)(Xl+(long)r*K+k0))[c] : make_int4(0,0,0,0);
      sH[r][c*4+0]=h.x; sH[r][c*4+1]=h.y; sH[r][c*4+2]=h.z; sH[r][c*4+3]=h.w;
      sL[r][c*4+0]=l.x; sL[r][c*4+1]=l.y; sL[r][c*4+2]=l.z; sL[r][c*4+3]=l.w;
    }
    for(int idx=tid; idx<TI*(MMQ_KC/32); idx+=256){
      int r=idx/(MMQ_KC/32), c=idx%(MMQ_KC/32); int o=i0+r;
      sWs[r][c] = (o<N && k0/32+c<KB) ? ws[(long)o*KB + k0/32 + c] : __float2half(0.f);
    }
    for(int idx=tid; idx<MMQ_TJ*(MMQ_KC/32); idx+=256){
      int r=idx/(MMQ_KC/32), c=idx%(MMQ_KC/32);
      sXs[r][c] = (r<B && k0/32+c<KB) ? Xs[(long)r*KB + k0/32 + c] : 0.f;
    }
    __syncthreads();
    #pragma unroll
    for(int blk=0; blk<MMQ_KC/32; ++blk){
      #pragma unroll
      for(int a=0;a<MMQ_RI;a++){
        float wsb = __half2float(sWs[ir0+a][blk]);
        #pragma unroll
        for(int b=0;b<MMQ_RJ;b++){
          int dh=0, dl=0;
          #pragma unroll
          for(int q=0;q<8;q++){
            int w = sW[ir0+a][blk*8+q];
            dh = __dp4a(w, sH[jr0+b][blk*8+q], dh);
            dl = __dp4a(w, sL[jr0+b][blk*8+q], dl);
          }
          acc[a][b] += (MMQ_ACC)(((float)dh + (float)dl*(1.0f/127.0f)) * (wsb * sXs[jr0+b][blk]));
        }
      }
    }
    __syncthreads();
  }
  #pragma unroll
  for(int a=0;a<MMQ_RI;a++){ int o=i0+ir0+a; if(o>=N) continue;
    #pragma unroll
    for(int b=0;b<MMQ_RJ;b++){ int j=jr0+b; if(j>=B) continue; Y[(long)j*YS+o]=(float)acc[a][b]; } }
}


// ══ THE SAME matmul, LOWERED AS A GEMM ══════════════════════════════════════════════════════════
// At batch 1 a projection is a GEMV and one warp per output row is the right shape. At batch B it is a GEMM,
// and that shape becomes wrong: the register-tiled form issues ~64 dp4a per 512-weight block and re-reads
// every sequence's activation slab through L2, which measured 128 GB/s of an available 3000 at B=32 — compute
// bound, tensor cores idle, and 3.76x behind llama.cpp, which switches to its MMQ kernels for exactly this.
//
// Nothing about the model changes. The `.lg` still says `matmul`; the COMPILER picks the lowering from the
// shape it is given. That is the same decision llama.cpp makes between mmvq and mmq, made in the emitter
// instead of by hand.
//
// Golden survives because the int8 dot product accumulates in int32 — exact, no rounding — and the per-32
// block scales are applied in registers in the SAME float grouping the dp4a path uses
// (`(float)c * (ws * xs)` per k-block, summed in k order). The arithmetic that decides a token is unchanged.
// Tile shape and K-step follow llama.cpp's MMQ configuration (ggml/src/ggml-cuda/mmq.cuh,
// mmq-config-ampere.cuh): MMQ_ITER_K = 256, 8 warps, shared-memory tiles padded to dodge bank conflicts.
// llama.cpp is MIT-licensed, Copyright (c) 2023-2024 The ggml authors; the DESIGN PARAMETERS below are taken
// from it, the code is ours. Attribution recorded in benchmarks/results/qwen38_27b.json.
//
// Why it matters, measured: our first version staged only 32 K-elements per __syncthreads round, so a
// K=5120 projection paid 320 barriers per output tile where llama.cpp pays 40. Same tensor-core instruction,
// 8x the synchronization. Staging 256 K per round is the single structural difference.
#define TBM 16
#define TBN 64
#define KSTEP 256
__global__ void bq_gemm_mma(float* __restrict__ Y, const signed char* __restrict__ Wq,
                            const __half* __restrict__ Ws, const signed char* __restrict__ Xq,
                            const float* __restrict__ Xs, int K, int N, int M, int YS){
  int KB=K/32; int mblk=blockIdx.y*TBM, nblk=blockIdx.x*TBN;
  int warp=threadIdx.x>>5, lane=threadIdx.x&31, tid=threadIdx.x; int gid=lane>>2, tq=lane&3;
  // +4 bytes of row padding: 32-byte rows make every lane of a quarter-warp hit the same shared-memory bank.
  const int XP = KSTEP + 4, WP = KSTEP + 4;
  extern __shared__ signed char smem[];
  signed char* Xsm = smem;                       // TBM rows x XP
  signed char* Wsm = smem + TBM*XP;              // TBN rows x WP
  float d0=0.f,d1=0.f,d2=0.f,d3=0.f;
  for(int k0=0;k0<K;k0+=KSTEP){
    // Stage KSTEP of K at once — one barrier pair per 256 elements instead of per 32.
    for(int i=tid;i<TBM*KSTEP;i+=256){ int r=i/KSTEP,c=i%KSTEP;
      Xsm[r*XP+c] = (mblk+r<M && k0+c<K) ? Xq[(long)(mblk+r)*K+k0+c] : (signed char)0; }
    for(int i=tid;i<TBN*KSTEP;i+=256){ int r=i/KSTEP,c=i%KSTEP;
      Wsm[r*WP+c] = (nblk+r<N && k0+c<K) ? Wq[(long)(nblk+r)*K+k0+c] : (signed char)0; }
    __syncthreads();
    // Then run the mma chain over the staged block with no further barriers.
    for(int ks=0; ks<KSTEP && k0+ks<K; ks+=32){
      int kb=(k0+ks)>>5;
      int a0=*(const int*)(Xsm+gid*XP+ks+tq*4);
      int a1=*(const int*)(Xsm+(gid+8)*XP+ks+tq*4);
      int a2=*(const int*)(Xsm+gid*XP+ks+16+tq*4);
      int a3=*(const int*)(Xsm+(gid+8)*XP+ks+16+tq*4);
      int b0=*(const int*)(Wsm+(warp*8+gid)*WP+ks+tq*4);
      int b1=*(const int*)(Wsm+(warp*8+gid)*WP+ks+16+tq*4);
      int c0=0,c1=0,c2=0,c3=0;
      asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 {%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};"
        : "+r"(c0),"+r"(c1),"+r"(c2),"+r"(c3) : "r"(a0),"r"(a1),"r"(a2),"r"(a3), "r"(b0),"r"(b1));
      int r0=mblk+gid, r1=mblk+gid+8, cbase=nblk+warp*8;
      float xs0 = (r0<M) ? Xs[(long)r0*KB+kb] : 0.f;
      float xs1 = (r1<M) ? Xs[(long)r1*KB+kb] : 0.f;
      float ws0 = (cbase+tq*2   <N) ? __half2float(Ws[(long)(cbase+tq*2)*KB+kb])   : 0.f;
      float ws1 = (cbase+tq*2+1 <N) ? __half2float(Ws[(long)(cbase+tq*2+1)*KB+kb]) : 0.f;
      d0+=(float)c0*(ws0*xs0); d1+=(float)c1*(ws1*xs0);
      d2+=(float)c2*(ws0*xs1); d3+=(float)c3*(ws1*xs1);
    }
    __syncthreads();
  }
  int r0=mblk+gid, r1=mblk+gid+8, cc0=nblk+warp*8+tq*2, cc1=cc0+1;
  if(r0<M){ if(cc0<N) Y[(long)r0*YS+cc0]=d0; if(cc1<N) Y[(long)r0*YS+cc1]=d1; }
  if(r1<M){ if(cc0<N) Y[(long)r1*YS+cc0]=d2; if(cc1<N) Y[(long)r1*YS+cc1]=d3; }
}

// ── THE KERNEL THAT MAKES BATCHING WORTH IT ─────────────────────────────────────────────────────
// One warp owns one output row. It walks the row's weights ONCE, and for each 512-weight block it reuses the
// loaded `int4` across BTG sequences held in registers. Weight traffic is therefore independent of the batch:
// the same bytes that produced one token now produce BTG of them.
__global__ void bq_gemv_h(float* Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                          const signed char* __restrict__ Xq, const float* __restrict__ Xs,
                          int K, int N, int B, int YS){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return;
  int lane=threadIdx.x&31; int b0=blockIdx.y*BTG;
  const int4* w16=(const int4*)(wq+(long)o*K); const __half* wsr=ws+(long)o*(K/32);
  int nq=K/512, KB=K/32;
  float acc[BTG];
  #pragma unroll
  for(int t=0;t<BTG;t++) acc[t]=0.f;
  for(int c=0;c<nq;c++){
    int4 w=w16[c*32+lane]; int blk=c*16+lane/2; float wsb=__half2float(wsr[blk]);
    #pragma unroll
    for(int t=0;t<BTG;t++){
      int bb=b0+t; if(bb>=B) break;
      int4 xx=((const int4*)(Xq+(long)bb*K))[c*32+lane];
      int d=__dp4a(w.x,xx.x,0)+__dp4a(w.y,xx.y,0)+__dp4a(w.z,xx.z,0)+__dp4a(w.w,xx.w,0);
      acc[t] += (float)d * (wsb*Xs[(long)bb*KB+blk]);
    }
  }
  int done=nq*512;
  for(int t=0;t<BTG;t++){
    int bb=b0+t; if(bb>=B) break;
    for(int j=done+lane;j<K;j+=32){ int blk=j/32;
      acc[t] += (float)((int)Xq[(long)bb*K+j]*(int)wq[(long)o*K+j])
              * (__half2float(wsr[blk])*Xs[(long)bb*KB+blk]); }
  }
  for(int t=0;t<BTG;t++){
    int bb=b0+t; if(bb>=B) break; float a=acc[t];
    for(int s=16;s>0;s>>=1) a+=__shfl_down_sync(0xffffffffu,a,s);
    if(lane==0) Y[(long)bb*YS+o]=a;
  }
}

// The two-pass activation form, batched the same way: the weight is still read once per row for the whole
// batch, and each sequence contributes two dp4a instead of one.
__global__ void bq_gemv_hl(float* Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
                           const signed char* __restrict__ Xh, const signed char* __restrict__ Xl,
                           const float* __restrict__ Xs, int K, int N, int B, int YS){
  int o=(blockIdx.x*blockDim.x+threadIdx.x)>>5; if(o>=N) return;
  int lane=threadIdx.x&31; int b0=blockIdx.y*BTG;
  const int4* w16=(const int4*)(wq+(long)o*K); const __half* wsr=ws+(long)o*(K/32);
  int nq=K/512, KB=K/32;
  float acc[BTG];
  #pragma unroll
  for(int t=0;t<BTG;t++) acc[t]=0.f;
  for(int c=0;c<nq;c++){
    int4 w=w16[c*32+lane]; int blk=c*16+lane/2; float wsb=__half2float(wsr[blk]);
    #pragma unroll
    for(int t=0;t<BTG;t++){
      int bb=b0+t; if(bb>=B) break;
      int4 hh=((const int4*)(Xh+(long)bb*K))[c*32+lane];
      int4 ll=((const int4*)(Xl+(long)bb*K))[c*32+lane];
      int dh=__dp4a(w.x,hh.x,0)+__dp4a(w.y,hh.y,0)+__dp4a(w.z,hh.z,0)+__dp4a(w.w,hh.w,0);
      int dl=__dp4a(w.x,ll.x,0)+__dp4a(w.y,ll.y,0)+__dp4a(w.z,ll.z,0)+__dp4a(w.w,ll.w,0);
      acc[t] += ((float)dh + (float)dl*(1.0f/127.0f)) * (wsb*Xs[(long)bb*KB+blk]);
    }
  }
  int done=nq*512;
  for(int t=0;t<BTG;t++){
    int bb=b0+t; if(bb>=B) break;
    for(int j=done+lane;j<K;j+=32){ int blk=j/32;
      float a=(float)Xh[(long)bb*K+j] + (float)Xl[(long)bb*K+j]*(1.0f/127.0f);
      acc[t] += a*(float)wq[(long)o*K+j]*(__half2float(wsr[blk])*Xs[(long)bb*KB+blk]); }
  }
  for(int t=0;t<BTG;t++){
    int bb=b0+t; if(bb>=B) break; float a=acc[t];
    for(int s=16;s>0;s>>=1) a+=__shfl_down_sync(0xffffffffu,a,s);
    if(lane==0) Y[(long)bb*YS+o]=a;
  }
}

// The tiny f32-weight projection GDN needs for its gates. Same shape, no quantization.
// ── THE GDN's SMALL f32 PROJECTION, ROW-PARALLEL ────────────────────────────────────────────────
// Third instance of the same shape mistake, and the clearest: one warp per output row, with only
// N = 2*v_heads = 96 rows. That is 96 warps per sequence — 192 warps, ~6k threads, on a 132-SM part
// — each grinding a K=5120 reduction 32 lanes at a time. Measured 0.45 ms over 48 launches.
//
// F32W_NW warps now share each row's reduction and rejoin through shared memory, the same fix as
// `bq_gemv_wide` and for the same reason. Unlike that one it is NOT bit-identical: splitting a
// reduction across warps re-associates it. The GDN gates it feeds are a sigmoid and a softplus decay,
// both smooth and both far from a decision boundary, so a one-ulp move there cannot change a token —
// but it is a different number, so it is asserted against the original within a bound rather than
// claimed equal.
#ifndef F32W_NW
#define F32W_NW 8
#endif
__global__ void __launch_bounds__(256) bq_gemv_f32w(float* Y, const float* __restrict__ w,
                             const float* __restrict__ X, int K, int N, int B){
  const int wpr=F32W_NW;                       // warps cooperating on one row
  const int rows_per_block=(256/32)/wpr;       // rows a block covers
  const int lane=threadIdx.x&31, warp=threadIdx.x>>5;
  const int o=blockIdx.x*rows_per_block + warp/wpr;
  const int sub=warp%wpr;
  const int bb=blockIdx.y; if(bb>=B) return;
  __shared__ float red[256/32];
  float a=0.f;
  if(o<N){ const float* wr=w+(long)o*K; const float* x=X+(long)bb*K;
    for(int j=lane+sub*32;j<K;j+=32*wpr) a+=wr[j]*x[j]; }
  for(int s=16;s>0;s>>=1) a+=__shfl_down_sync(0xffffffffu,a,s);
  if(lane==0) red[warp]=a;
  __syncthreads();
  if(o<N && sub==0 && lane==0){
    float t=0.f;
    for(int q=0;q<wpr;q++) t+=red[(warp/wpr)*wpr+q];   // fixed ascending order: deterministic
    Y[(long)bb*N+o]=t;
  }
}

// ── the per-sequence pointwise and reduction kernels: one block per sequence ─────────────────────
__global__ void bq_embed(float* X, const signed char* wq, const __half* ws, const int* D_tok, int E){
  int bb=blockIdx.y; long tok=D_tok[bb];
  for(int t=blockIdx.x*blockDim.x+threadIdx.x; t<E; t+=gridDim.x*blockDim.x)
    X[(long)bb*E+t]=(float)wq[tok*E+t]*__half2float(ws[tok*(E/32)+t/32]);
}
__global__ void bq_rmsnorm_q8f_hl(const float* X, signed char* Xq, signed char* Xl, float* Xs,
                                  float* Xf, const float* w, int n, float eps){
  int bb=blockIdx.x; const float* x=X+(long)bb*n; float* xf=Xf+(long)bb*n;
  __shared__ float sh[RMSN_NT];
  float s=0.f; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=x[i]*x[i];
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/n+eps); __syncthreads();
  for(int i=threadIdx.x;i<n;i+=blockDim.x) xf[i]=x[i]*sc*w[i];
  __syncthreads();
  int nb=n/32;
  for(int b=threadIdx.x;b<nb;b+=blockDim.x){
    float amax=1e-30f;
    for(int j=0;j<32;++j){ float a=fabsf(xf[b*32+j]); if(a>amax) amax=a; }
    float q=amax/127.0f; Xs[(long)bb*nb+b]=q;
    for(int j=0;j<32;++j) lgx_q8hl(xf[b*32+j]/q, Xq+(long)bb*n, Xl+(long)bb*n, (long)b*32+j);
  }
}
// The residual-add + RMSNorm + requantise that follows a projection. `SK` folds the split-K
// reduction in: instead of reading a GEMM output some earlier kernel summed, it sums the S partials
// itself, in the SAME order `bq_sk_reduce` used (0.f, then s ascending), so the float result is
// bit-identical -- and the separate launch, plus a full write and read of Y, disappear.
template<bool SK>
__device__ __forceinline__ void bq_add_rmsnorm_q8f_hl_body(float* X, const float* Y, int S,
                                      signed char* Xq, signed char* Xl,
                                      float* Xs, float* Xf, const float* w, int n, float eps){
  int bb=blockIdx.x; float* x=X+(long)bb*n; const float* y=Y+(long)bb*n; float* xf=Xf+(long)bb*n;
  __shared__ float sh[RMSN_NT];
  const long skst=(long)n*gridDim.x;
  float s=0.f; for(int i=threadIdx.x;i<n;i+=blockDim.x){
    float yv; if(SK){ yv=0.f; for(int q=0;q<S;q++) yv+=y[(long)q*skst+i]; } else yv=y[i];
    float v=x[i]+yv; x[i]=v; s+=v*v; }
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/n+eps); __syncthreads();
  for(int i=threadIdx.x;i<n;i+=blockDim.x) xf[i]=x[i]*sc*w[i];
  __syncthreads();
  int nb=n/32;
  for(int b=threadIdx.x;b<nb;b+=blockDim.x){
    float amax=1e-30f;
    for(int j=0;j<32;++j){ float a=fabsf(xf[b*32+j]); if(a>amax) amax=a; }
    float q=amax/127.0f; Xs[(long)bb*nb+b]=q;
    for(int j=0;j<32;++j) lgx_q8hl(xf[b*32+j]/q, Xq+(long)bb*n, Xl+(long)bb*n, (long)b*32+j);
  }
}
__global__ void bq_add_rmsnorm_q8f_hl(float* X, const float* Y, signed char* Xq, signed char* Xl,
                                      float* Xs, float* Xf, const float* w, int n, float eps){
  bq_add_rmsnorm_q8f_hl_body<false>(X,Y,1,Xq,Xl,Xs,Xf,w,n,eps);
}
__global__ void bq_add_rmsnorm_q8f_hl_sk(float* X, const float* P, int S, signed char* Xq, signed char* Xl,
                                         float* Xs, float* Xf, const float* w, int n, float eps){
  bq_add_rmsnorm_q8f_hl_body<true>(X,P,S,Xq,Xl,Xs,Xf,w,n,eps);
}
__global__ void bq_add(float* X, const float* Y, int n){
  int bb=blockIdx.y; int i=blockIdx.x*blockDim.x+threadIdx.x;
  if(i<n) X[(long)bb*n+i]+=Y[(long)bb*n+i];
}
__global__ void bq_gate_mul(float* A, int as_, const float* G, int gs, int n){
  int bb=blockIdx.y; int i=blockIdx.x*blockDim.x+threadIdx.x;
  if(i<n){ float g=G[(long)bb*gs+i]; long o=(long)bb*as_+i; A[o]*= 1.0f/(1.0f+__expf(-g)); }
}
// The attention tail: gate, then requantise. Two launches over the SAME grid, the second reading back
// what the first wrote to global memory one instruction earlier. Fused, the gated value never leaves a
// register, and the amax reduction the quantiser already does covers it. Bit-identical: the expression
// producing `v` is unchanged, and `A` is still written because the next step's oracle reads it.
__global__ void bq_gate_mul_q8_hl(float* A, int as_, const float* G, int gs,
                                  signed char* Xq, signed char* Xl, float* Xs, int K){
  int bb=blockIdx.y;
  int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if((long)blk*32>=K) return; int lane=threadIdx.x&31;
  const long i=(long)blk*32+lane;
  const float g=G[(long)bb*gs+i];
  const long o=(long)bb*as_+i;
  const float v=A[o]*(1.0f/(1.0f+__expf(-g)));
  A[o]=v;
  float a=fabsf(v);
  for(int s=16;s>0;s>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,s)); a=__shfl_sync(0xffffffffu,a,0);
  float sc=a>0?a/127.0f:1.0f; if(lane==0) Xs[(long)bb*(K/32)+blk]=sc;
  lgx_q8hl(v/sc, Xq+(long)bb*K, Xl+(long)bb*K, i);
}
__global__ void bq_quant_act_hl(const float* X, signed char* Xq, signed char* Xl, float* Xs, int K){
  int bb=blockIdx.y;
  int blk=blockIdx.x*(blockDim.x/32)+threadIdx.x/32; if((long)blk*32>=K) return; int lane=threadIdx.x&31;
  const float* x=X+(long)bb*K;
  float v=x[blk*32+lane], a=fabsf(v);
  for(int s=16;s>0;s>>=1) a=fmaxf(a,__shfl_down_sync(0xffffffffu,a,s)); a=__shfl_sync(0xffffffffu,a,0);
  float sc=a>0?a/127.0f:1.0f; if(lane==0) Xs[(long)bb*(K/32)+blk]=sc;
  lgx_q8hl(v/sc, Xq+(long)bb*K, Xl+(long)bb*K, (long)blk*32+lane);
}
// ── THE SwiGLU EPILOGUE, PARALLELISED BY THE RIGHT UNIT ─────────────────────────────────────────
// This measured 1.14 ms of a 20.3 ms step at B=2 — 64 launches at 17.8 us — and the reason was a
// grid sized by the wrong quantity. The work unit is a 32-wide quantisation block, and the kernel
// gave one THREAD to each: `Gs(dff)` launches dff/256 blocks of 256 threads, so 17408 threads start
// and `if(blk>=nb) return` sends all but dff/32 = 544 of them home. 1.6% of the threads did the work,
// each looping 32 elements through a `float t[32]` register array.
//
// One WARP per 32-block instead of one thread fixes it: every lane owns one element, the block's
// amax comes from a shuffle reduction, and the same launch geometry now has every thread working.
//
// It is BIT-IDENTICAL, not a tolerance. `fmaxf` is associative and exact, so the shuffle reduction
// produces the same `amax` the serial loop did; `sc` is therefore the same float, and each lane then
// runs the identical quantisation on the identical value. Only which thread computes what has moved.
template<bool SK>
__device__ __forceinline__ void bq_silu_had_q8_hl_body(const float* F, int S, signed char* Hq, signed char* Hl, float* Hs, int n){
  const int bb=blockIdx.y, nb=n/32;
  const int warp=(blockIdx.x*blockDim.x+threadIdx.x)>>5, lane=threadIdx.x&31;
  if(warp>=nb) return;
  const float* g=F+(long)bb*2*n; const float* u=g+n;
  const long idx=(long)warp*32+lane;
  const long skst=(long)2*n*gridDim.y;
  float gv,uv;
  if(SK){ gv=0.f; uv=0.f; for(int q=0;q<S;q++){ gv+=g[(long)q*skst+idx]; uv+=u[(long)q*skst+idx]; } }
  else  { gv=g[idx]; uv=u[idx]; }
  const float h=(gv/(1.0f+__expf(-gv)))*uv;
  float a=fabsf(h);
  for(int s=16;s;s>>=1) a=fmaxf(a,__shfl_xor_sync(0xffffffffu,a,s));
  const float sc=fmaxf(a,1e-30f)/127.0f;          // the serial form seeded amax at 1e-30
  if(lane==0) Hs[(long)bb*nb+warp]=sc;
  lgx_q8hl(h/sc, Hq+(long)bb*n, Hl+(long)bb*n, idx);
}
__global__ void bq_silu_had_q8_hl(const float* F, signed char* Hq, signed char* Hl, float* Hs, int n){
  bq_silu_had_q8_hl_body<false>(F,1,Hq,Hl,Hs,n);
}
__global__ void bq_silu_had_q8_hl_sk(const float* P, int S, signed char* Hq, signed char* Hl, float* Hs, int n){
  bq_silu_had_q8_hl_body<true>(P,S,Hq,Hl,Hs,n);
}
// Greedy pick over R rows of logits, WITHOUT advancing any position. Decode's `bq_argmax_adv` folds
// the position bump into the pick because a step always consumes exactly one token; a speculative step
// does not know how many it consumed until the drafts have been checked against these very rows, so
// the two have to come apart. Same reduction tree and same tie-break (first index wins on >), so a row
// scored here picks the token decode would have picked.
// Lay the speculative chunk out the way a T>1 forward expects it: row t*B+b. Row 0 is the token the
// model actually produced last; row 1 is the draft being checked. Nothing else in the verify pass
// needs to know it is speculative -- it is an ordinary two-token chunk.
// The draft head conditions on the embedding of the token just produced AND the hidden state of the
// token BEFORE it. After a verify pass `x` holds both rows -- position p in row 0, p+1 in row 1 --
// and which is "the one before" depends on whether the draft was accepted. Leaving it at row 0 makes
// the draft condition on a state one token stale exactly when it was RIGHT. That diverges the stream
// from greedy at the second token, and every throughput measurement was blind to it.
__global__ void bq_mtp_pick_hidden(float* __restrict__ X, const int* __restrict__ Acc, int B, int E){
  int b=blockIdx.y; if(!Acc[b]) return;
  for(int i=blockIdx.x*blockDim.x+threadIdx.x; i<E; i+=gridDim.x*blockDim.x)
    X[(long)b*E+i] = X[(long)(B+b)*E+i];
}
__global__ void bq_mtp_pack(int* __restrict__ D_tok, const int* __restrict__ D_draft, int B){
  int b=blockIdx.x*blockDim.x+threadIdx.x; if(b>=B) return;
  D_tok[B+b]=D_draft[b];
}
// Accept per sequence: the draft stands only if the model's own pick at that position agrees.
// `V` holds the verify pass's greedy picks in the same row-major order, so V[b] is the TRUE token at
// position p+1 and V[B+b] is the token at p+2 that is only real if the draft was.
__global__ void bq_mtp_accept(const int* __restrict__ V, const int* __restrict__ D_draft,
                              int* __restrict__ D_tok, int* __restrict__ D_pos,
                              int* __restrict__ Acc, int B){
  int b=blockIdx.x*blockDim.x+threadIdx.x; if(b>=B) return;
  int ok = (V[b]==D_draft[b]) ? 1 : 0;
  Acc[b]  = ok;
  if(ok) atomicAdd(Acc+B, 1);        // Acc[B] is the running accept total
  D_tok[b]= ok ? V[B+b] : V[b];      // next step consumes p+2 on accept, p+1 on reject
  D_pos[b]= D_pos[b] + 1 + ok;
}
// Roll the recurrence back for the sequences whose draft was wrong. Accepted sequences keep the
// state the verify pass left (it consumed a token that turned out to be real); rejected ones take
// the checkpoint written after token 0, which the lock proves is exactly what a one-token chunk
// leaves. Copying only the rejected slices is why this is cheap at a high accept rate.
// ★ THE RECURRENT STATE'S STORAGE WIDTH. It is 151 MB per sequence — 9.66 GB of read-modify-write per
// decode step at B=32, and at 4.8 GB resident it is the largest term in the step that cannot hide in
// L2. Halving it halves that traffic AND doubles the concurrency ceiling, which is a serving metric of
// its own: vLLM carries the identical state and its baseline here died on "max_num_seqs exceeds
// available Mamba cache blocks".
//
// Ten mantissa bits, not eight. Measured against an f64 reference the output drifts ~1e-3 with f16 and
// 1.7e-2 to 2.6e-1 with bf16 — and the drift does NOT grow with sequence length, because the decay
// multiplies an error injected at step t by alpha^(T-t) before it reaches the output, so old error dies
// instead of accumulating. That is what licenses a half-width state at all; it is asserted by
// `the_gated_recurrence_tolerates_a_half_width_state_but_only_with_ten_mantissa_bits`.
//
// The body touches the state EXACTLY TWICE — one load into registers, one store at the end — so the
// width is a typedef and nothing else in the recurrence changes. Compute stays f32 throughout.
// ★ THE KV CACHE'S STORAGE WIDTH. The state lane alone does NOT double the concurrency ceiling, which
// is a claim this ledger had to retract: the GDN state is one of three per-sequence terms and at 2K
// context the KV cache is the LARGER of them, because it is kept as `float`. Per sequence at 2K:
// 151 MB state + 268 MB KV + 8 MB conv. Halving the state alone is 427 -> 352 MB (1.21x more
// sequences); halving both is 427 -> 218 MB (1.97x). Concurrency is a serving metric and vLLM carries
// the identical recurrent state, so this is the half that makes the pair worth having.
//
// f16 is conservative here — attention is a weighted average over many keys, production engines ship
// fp8 KV, and this keeps ten mantissa bits. Compute stays f32: only the cached representation narrows.
#ifdef BQ_KV16
typedef __half bq_kv_t;
__device__ __forceinline__ float bq_kv_ld(__half v){ return __half2float(v); }
__device__ __forceinline__ __half bq_kv_cv(float v){ return __float2half(v); }
#else
typedef float bq_kv_t;
__device__ __forceinline__ float bq_kv_ld(float v){ return v; }
__device__ __forceinline__ float bq_kv_cv(float v){ return v; }
#endif
#ifdef BQ_STATE16
typedef __half bq_state_t;
__device__ __forceinline__ float bq_st_ld(__half v){ return __half2float(v); }
__device__ __forceinline__ __half bq_st_cv(float v){ return __float2half(v); }
#else
typedef float bq_state_t;
__device__ __forceinline__ float bq_st_ld(float v){ return v; }
__device__ __forceinline__ float bq_st_cv(float v){ return v; }
#endif
__global__ void bq_mtp_rollback(bq_state_t* __restrict__ S, const bq_state_t* __restrict__ SM,
                                const int* __restrict__ Acc, long per_seq){
  int b=blockIdx.y; if(Acc[b]) return;
  long i=(long)blockIdx.x*blockDim.x+threadIdx.x, stride=(long)gridDim.x*blockDim.x;
  bq_state_t* d=S+(long)b*per_seq; const bq_state_t* src=SM+(long)b*per_seq;
  for(; i<per_seq; i+=stride) d[i]=src[i];
}
// ── PRE-TILED WEIGHTS ────────────────────────────────────────────────────────────────────────────
// The decode GEMM waits 67% of its time for data (measured in-kernel with clock64(), since ncu is
// permission-blocked in RunPod containers) while sitting at 42% of a 3180 GB/s pure-read roofline.
// Neither depth, occupancy, run length, tile shape nor the instruction moved it, because none of them
// change the ACCESS PATTERN: row-major weights make a block read TI separate KC-byte streams, and a
// reader that streams contiguously gets 95% of the roofline.
//
// Storing each (n-tile, k-chunk) contiguously turns that into ONE streaming read of TI*KC bytes. The
// transform runs once at load: it is a pure permutation of the same bytes, so the arithmetic is
// untouched and the equivalence lock must still see bit-identity.
__global__ void bq_tile_w(signed char* __restrict__ dst, const signed char* __restrict__ src,
                          int N, int K, int TI, int KC){
  long tiles_k = (long)((K + KC - 1) / KC);
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x, stride = (long)gridDim.x * blockDim.x;
  long total = (long)((N + TI - 1) / TI) * tiles_k * TI * KC;
  for(; i < total; i += stride){
    long within = i % ((long)TI * KC);
    long tile   = i / ((long)TI * KC);
    int  r  = (int)(within / KC), c = (int)(within % KC);
    int  nt = (int)(tile / tiles_k), kt = (int)(tile % tiles_k);
    int  n = nt * TI + r, k = kt * KC + c;
    dst[i] = (n < N && k < K) ? src[(long)n * K + k] : (signed char)0;
  }
}
// The same permutation for the per-32 scales, which are read on the same footing.
__global__ void bq_tile_ws(__half* __restrict__ dst, const __half* __restrict__ src,
                           int N, int K, int TI, int KC){
  int NB = K / 32, NBC = KC / 32;
  long tiles_k = (long)((K + KC - 1) / KC);
  long i = (long)blockIdx.x * blockDim.x + threadIdx.x, stride = (long)gridDim.x * blockDim.x;
  long total = (long)((N + TI - 1) / TI) * tiles_k * TI * NBC;
  for(; i < total; i += stride){
    long within = i % ((long)TI * NBC);
    long tile   = i / ((long)TI * NBC);
    int  r  = (int)(within / NBC), c = (int)(within % NBC);
    int  nt = (int)(tile / tiles_k), kt = (int)(tile % tiles_k);
    int  n = nt * TI + r, b = kt * NBC + c;
    dst[i] = (n < N && b < NB) ? src[(long)n * NB + b] : __float2half(0.f);
  }
}
// ── THE fp8 LANE ─────────────────────────────────────────────────────────────────────────────────
// Q8_0 carries a scale every 32 elements, so the accumulator must be read out, converted and rescaled
// FOUR times per mma issue. fp8 e4m3 accumulates in fp32 natively and takes ONE scale per output
// channel at the very end -- which is what vLLM's fp8 path does, so this is the matched-precision
// comparison rather than a cheaper one.
//
// The register shape is IDENTICAL to the int8 m16n8k32 the tile already uses (4 A regs, 2 B regs,
// 4 accumulators), so the fragment map, the swizzle and the staging survive untouched: only the
// instruction, the accumulator type and where the scale lands change. fp8 mma needs PTX ISA 8.1
// (CUDA 12.1+); the box has 12.8 at /usr/local/cuda-12.8 while `nvcc` on PATH is 12.0.
// fp8 needs PTX ISA 8.1 (CUDA 12.1+) to assemble and <cuda_fp8.h> to convert. The equivalence locks
// compile with whatever `nvcc` is on PATH, which on this box is 12.0, so BOTH the header and the
// instruction are guarded and an older toolkit still builds the whole kernel set -- it simply has no
// fp8 lane. `__CUDA_ARCH__ >= 890` is the arch gate: Ada and Hopper have fp8 tensor cores, Ampere
// does not, which is also what chip.rs has said since the start.
#if (__CUDACC_VER_MAJOR__ > 12) || (__CUDACC_VER_MAJOR__ == 12 && __CUDACC_VER_MINOR__ >= 1)
#include <cuda_fp8.h>
#define BQ_FP8_TOOLKIT 1
#else
#define BQ_FP8_TOOLKIT 0
#endif
#if BQ_FP8_TOOLKIT && defined(__CUDA_ARCH__) && __CUDA_ARCH__ >= 890
#define BQ_HAVE_FP8 1
#else
#define BQ_HAVE_FP8 0
#endif
__device__ __forceinline__ void bq_mma_e4m3(float& d0, float& d1, float& d2, float& d3,
                                            unsigned a0, unsigned a1, unsigned a2, unsigned a3,
                                            unsigned b0, unsigned b1){
#if BQ_HAVE_FP8
  asm volatile("mma.sync.aligned.m16n8k32.row.col.f32.e4m3.e4m3.f32 "
               "{%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};\n"
               : "+f"(d0),"+f"(d1),"+f"(d2),"+f"(d3)
               : "r"(a0),"r"(a1),"r"(a2),"r"(a3),"r"(b0),"r"(b1));
#else
  (void)a0;(void)a1;(void)a2;(void)a3;(void)b0;(void)b1;
#endif
}
// Q8_0 -> e4m3 with ONE scale per output channel. The real weight is q*s_blk; the channel scale is
// the largest magnitude in the row over 448 (e4m3's max finite), so the row uses the full range.
// This is a REQUANTISATION and it loses precision on purpose -- int8 carries 8 mantissa bits where
// e4m3 carries 4 -- which is exactly the trade vLLM already makes and which the accuracy column
// against the f64 oracle has to price rather than assume.
__global__ void bq_q8_to_e4m3_scale(float* __restrict__ chan_scale,
                                    const signed char* __restrict__ q, const __half* __restrict__ s,
                                    int N, int K){
  int n = blockIdx.x; if(n >= N) return;
  int KB = K/32;
  __shared__ float red[256];
  float m = 0.f;
  for(int k = threadIdx.x; k < K; k += blockDim.x){
    float w = (float)q[(long)n*K + k] * __half2float(s[(long)n*KB + (k>>5)]);
    m = fmaxf(m, fabsf(w));
  }
  red[threadIdx.x] = m; __syncthreads();
  for(int t = blockDim.x/2; t > 0; t >>= 1){
    if(threadIdx.x < t) red[threadIdx.x] = fmaxf(red[threadIdx.x], red[threadIdx.x+t]);
    __syncthreads(); }
  if(threadIdx.x == 0) chan_scale[n] = (red[0] > 0.f) ? (red[0] / 448.0f) : 1.0f;
}
__global__ void bq_q8_to_e4m3(unsigned char* __restrict__ out, const float* __restrict__ chan_scale,
                              const signed char* __restrict__ q, const __half* __restrict__ s,
                              int N, int K){
  int KB = K/32;
  long i = (long)blockIdx.x*blockDim.x + threadIdx.x, stride = (long)gridDim.x*blockDim.x;
  for(; i < (long)N*K; i += stride){
    int n = (int)(i / K), k = (int)(i % K);
    float w = (float)q[i] * __half2float(s[(long)n*KB + (k>>5)]);
#if BQ_FP8_TOOLKIT
    out[i] = (unsigned char)__nv_cvt_float_to_fp8(w / chan_scale[n], __NV_SATFINITE, __NV_E4M3);
#else
    out[i] = 0;                      // no fp8 toolkit: the lane is absent, not silently wrong
#endif
  }
}
__global__ void bq_argmax_rows(const float* L, int V, int* D_tok){
  int bb=blockIdx.x; const float* l=L+(long)bb*V;
  __shared__ float bv[256]; __shared__ int bi[256];
  float best=-1e30f; int arg=0;
  for(int i=threadIdx.x;i<V;i+=blockDim.x) if(l[i]>best){ best=l[i]; arg=i; }
  bv[threadIdx.x]=best; bi[threadIdx.x]=arg; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){
    if(threadIdx.x<k && bv[threadIdx.x+k]>bv[threadIdx.x]){ bv[threadIdx.x]=bv[threadIdx.x+k]; bi[threadIdx.x]=bi[threadIdx.x+k]; }
    __syncthreads(); }
  if(threadIdx.x==0) D_tok[bb]=bi[0];
}
__global__ void bq_argmax_adv(const float* L, int V, int* D_tok, int* D_pos){
  int bb=blockIdx.x; const float* l=L+(long)bb*V;
  __shared__ float bv[256]; __shared__ int bi[256];
  float best=-1e30f; int arg=0;
  for(int i=threadIdx.x;i<V;i+=blockDim.x) if(l[i]>best){ best=l[i]; arg=i; }
  bv[threadIdx.x]=best; bi[threadIdx.x]=arg; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){
    if(threadIdx.x<k && bv[threadIdx.x+k]>bv[threadIdx.x]){ bv[threadIdx.x]=bv[threadIdx.x+k]; bi[threadIdx.x]=bi[threadIdx.x+k]; }
    __syncthreads(); }
  if(threadIdx.x==0){ D_tok[bb]=bi[0]; D_pos[bb]++; }
}

// ── the hybrid's stateful halves, one sequence per grid.z ────────────────────────────────────────
// Every sequence carries its own recurrence memory, conv ring and KV cache. That per-sequence state, not the
// weights, is what bounds the batch: the weights are shared by everyone, the state is not.
__global__ void bq_conv_silu_l2(const float* __restrict__ X, int xs, const float* __restrict__ wc,
                                float* __restrict__ CS, float* __restrict__ Y,
                                int C, int T, int nl2, float eps){
  int bb=blockIdx.y;
  const float* x=X+(long)bb*xs; float* cs=CS+(long)bb*T*C; float* y=Y+(long)bb*C;
  int c=blockIdx.x*blockDim.x+threadIdx.x; if(c>=C) return;
  for(int t=0;t<T-1;++t) cs[(long)t*C+c]=cs[(long)(t+1)*C+c];
  cs[(long)(T-1)*C+c]=x[c];
  float acc=0.f;
  for(int t=0;t<T;++t) acc += cs[(long)t*C+c]*wc[(long)t*C+c];
  float v=acc/(1.0f+__expf(-acc));
  if(blockIdx.x>=nl2){ y[c]=v; return; }
  __shared__ float ss[32];
  int lane=threadIdx.x&31, warp=threadIdx.x>>5, nw=(blockDim.x+31)>>5;
  float pp=gdn_warp_sum(v*v);
  if(lane==0) ss[warp]=pp;
  __syncthreads();
  __shared__ float den;
  if(threadIdx.x==0){ float t2=0.f; for(int j=0;j<nw;++j) t2+=ss[j]; den=sqrtf(t2+eps); }
  __syncthreads();
  y[c]=v/den;
}
__global__ void bq_gdn_gates(const float* __restrict__ G, const float* __restrict__ dt,
                             const float* __restrict__ alog, float* __restrict__ beta,
                             float* __restrict__ decay, int NV){
  int bb=blockIdx.y; int i=blockIdx.x*blockDim.x+threadIdx.x; if(i>=NV) return;
  const float* br=G+(long)bb*2*NV; const float* ar=br+NV;
  beta[(long)bb*NV+i]  = 1.0f/(1.0f+__expf(-br[i]));
  decay[(long)bb*NV+i] = powf(1.0f+__expf(ar[i]+dt[i]), alog[i]);
}
__global__ void bq_gdn_step(const float* __restrict__ Q, const float* __restrict__ Kk, const float* __restrict__ V,
                            const float* __restrict__ BET, const float* __restrict__ ALP,
                            bq_state_t* __restrict__ S, float* __restrict__ O, int D, int NK, int NV, int CS){
  const unsigned m=0xffffffffu;
  int bb=blockIdx.z;
  int h=blockIdx.x, lane=threadIdx.x&31, warp=threadIdx.x>>5;
  int a=blockIdx.y*(blockDim.x>>5)+warp; if(a>=D) return;
  int g=h%NK;
  const float* kk=Kk+(long)bb*CS+(long)g*D;
  const float* qq=Q +(long)bb*CS+(long)g*D;
  bq_state_t* Sb=S+(long)bb*NV*D*D;
  long base=(long)h*D*D+(long)a*D;
  float sreg[8]; float u_=0.f,w_=0.f,kq_=0.f; int n=0;
  for(int b=lane;b<D;b+=32){
    float sv=Sb[base+b], kb=kk[b], qb=qq[b];
    sreg[n++]=sv; u_+=sv*kb; w_+=sv*qb; kq_+=kb*qb;
  }
  for(int sh=16;sh>0;sh>>=1){ u_+=__shfl_down_sync(m,u_,sh); w_+=__shfl_down_sync(m,w_,sh); kq_+=__shfl_down_sync(m,kq_,sh); }
  u_=__shfl_sync(m,u_,0); w_=__shfl_sync(m,w_,0); kq_=__shfl_sync(m,kq_,0);
  float alpha=ALP[(long)bb*NV+h], beta=BET[(long)bb*NV+h];
  float coef=beta*(V[(long)bb*CS+(long)h*D+a] - alpha*u_);
  if(lane==0) O[(long)bb*NV*D+(long)h*D+a]=(alpha*w_+coef*kq_)*rsqrtf((float)D);
  n=0; for(int b=lane;b<D;b+=32) Sb[base+b]=alpha*sreg[n++]+coef*kk[b];
}
#ifndef BQ_GDN_DELTA_TMAX
#define BQ_GDN_DELTA_TMAX 64
#endif
// ── THE CHUNKED DELTA RULE — parallel across the chunk ───────────────────────────────────────────
// `bq_gdn_chunk` is a SCAN: every token needs the state the previous token left, so T tokens serialise
// with D^2 work apiece. Measured, that is prefill's dominant cost (0.1811 ms/layer/token => ~4.5 s over
// 48 layers and a 512-token prompt), and no amount of state-residency fixes the dependency itself.
//
// The same recurrence closes in a unit-triangular solve. With A_t = prod(alpha) and alpha_t*A_{t-1}=A_t:
//
//    c_t + SUM_{j<t} beta_t (A_t/A_j)(k_j.k_t) c_j = beta_t (v_t - A_t S0 k_t)      (I+M) C = B
//    O_t = [ A_t (S0 q_t) + SUM_{j<=t} (A_t/A_j)(k_j.q_t) C_j ] / sqrt(D)
//    S_C =   A_C S0 + SUM_j (A_C/A_j) C_j k_j^T
//
// Proved equal to the scan to machine epsilon in f64 by
// `the_chunked_delta_rule_equals_the_sequential_gated_recurrence` BEFORE this kernel was written, so a
// failure here is an implementation bug and never an algebra bug.
//
// The decisive property: every S0 term is INDEPENDENT across t, and in the C recursion the head
// dimension `a` is THREAD-LOCAL — no cross-lane reduction inside the serial part. The critical path
// falls from T*D^2 to T*D.
//
// v1 is written for correctness against the f64 oracle, not for speed: one block per (sequence, head),
// each thread owning whole rows of the state. Optimise only once it is proven equal.
// ★ TEMPLATED ON THE CHUNK LENGTH, and that is not a style choice. With `float proj[64]` sized by a
// runtime bound, ptxas reported **512 bytes stack frame** against the scan's 0 — the per-thread arrays
// were in LOCAL MEMORY, which is the identical trap `bq_gdn_chunk_body` was templated on `PER` to
// escape. Read the `-Xptxas -v` stack-frame line before believing any recurrence kernel is in registers.
template<int TT>
__device__ __forceinline__ void bq_gdn_delta_body(const float* __restrict__ Q, const float* __restrict__ Kk,
                             const float* __restrict__ V, const float* __restrict__ BET,
                             const float* __restrict__ ALP, bq_state_t* __restrict__ S,
                             float* __restrict__ O, int D, int NK, int NV, int CS, int T, int B){
  extern __shared__ float bqd_sh[];
  const int h = blockIdx.x, bb = blockIdx.y, g = h % NK;
  const int tid = threadIdx.x, nthr = blockDim.x;
  float* sKK = bqd_sh;                 // [T][T]  k_j . k_t
  float* sKQ = sKK + (long)T * T;      // [T][T]  k_j . q_t
  float* sA  = sKQ + (long)T * T;      // [T]     A_t = prod alpha
  const float inv_sqrt_d = rsqrtf((float)D);

  // A_t: T is small and this is the only serial-in-T step left in the kernel.
  if (tid == 0) {
    float run = 1.f;
    for (int t = 0; t < T; ++t) { run *= ALP[(long)(bb * T + t) * NV + h]; sA[t] = run; }
  }
  __syncthreads();

  // K and Q staged once: the Gram matrices and every S0 contraction below read them repeatedly.
  float* sK = sA + T;                  // [T][D]
  float* sQ = sK + (long)T * D;        // [T][D]
  for (int idx = tid; idx < T * D; idx += nthr) {
    const int t = idx / D, b = idx % D;
    sK[idx] = Kk[(long)(bb * T + t) * CS + (long)g * D + b];
    sQ[idx] = Q [(long)(bb * T + t) * CS + (long)g * D + b];
  }
  __syncthreads();

  // The two Gram matrices, one thread per (t,j) pair.
  for (int idx = tid; idx < T * T; idx += nthr) {
    const int t = idx / T, j = idx % T;
    float akk = 0.f, akq = 0.f;
    for (int b = 0; b < D; ++b) {
      const float kj = sK[(long)j * D + b];
      akk += kj * sK[(long)t * D + b];
      akq += kj * sQ[(long)t * D + b];
    }
    sKK[idx] = akk; sKQ[idx] = akq;
  }
  __syncthreads();

  // One thread owns a whole state row `a`, so the C recursion needs no cross-lane reduction.
  //
  // ★ The b-loop is OUTERMOST in every phase. v1 put it inside the t-loop and so re-read the S0 row
  //   from global 2T+1 = 65 times per row; that alone made it 12x SLOWER than the scan it replaces
  //   (2.1351 vs 0.1812 ms/layer/token). The arithmetic is identical either way — this is purely how
  //   many times the same bytes are fetched.
  for (int a = tid; a < D; a += nthr) {
    const bq_state_t* s0row = S + (long)bb * NV * D * D + (long)h * D * D + (long)a * D;
    float proj[TT];                    // reused: S0.k_t first, then S0.q_t
    float cvec[TT];

    // Every loop that touches proj/cvec is bounded by TT and fully unrolled, so their indices are
    // compile-time constants and the arrays live in REGISTERS. With runtime bounds they were
    // addressable and ptxas put them in local memory (272 bytes of frame) no matter how the kernel
    // was templated -- templating alone is not enough, the INDICES have to be constant too.
    #pragma unroll
    for (int t = 0; t < TT; ++t) proj[t] = 0.f;
    for (int b = 0; b < D; ++b) {
      const float sv = bq_st_ld(s0row[b]);
      #pragma unroll
      for (int t = 0; t < TT; ++t) proj[t] += sv * sK[(long)t * D + b];
    }
    #pragma unroll
    for (int t = 0; t < TT; ++t) {
      const float beta = BET[(long)(bb * TT + t) * NV + h];
      const float va = V[(long)(bb * TT + t) * CS + (long)h * D + a];
      float acc = beta * (va - sA[t] * proj[t]);
      #pragma unroll
      for (int j = 0; j < TT; ++j)
        if (j < t) acc -= beta * (sA[t] / sA[j]) * sKK[t * TT + j] * cvec[j];
      cvec[t] = acc;
    }

    #pragma unroll
    for (int t = 0; t < TT; ++t) proj[t] = 0.f;
    for (int b = 0; b < D; ++b) {
      const float sv = bq_st_ld(s0row[b]);
      #pragma unroll
      for (int t = 0; t < TT; ++t) proj[t] += sv * sQ[(long)t * D + b];
    }
    #pragma unroll
    for (int t = 0; t < TT; ++t) {
      float acc = sA[t] * proj[t];
      #pragma unroll
      for (int j = 0; j < TT; ++j)
        if (j <= t) acc += (sA[t] / sA[j]) * sKQ[t * TT + j] * cvec[j];
      O[(long)(bb * TT + t) * NV * D + (long)h * D + a] = acc * inv_sqrt_d;
    }

    bq_state_t* srow = S + (long)bb * NV * D * D + (long)h * D * D + (long)a * D;
    for (int b = 0; b < D; ++b) {
      float acc = sA[TT - 1] * bq_st_ld(srow[b]);
      #pragma unroll
      for (int j = 0; j < TT; ++j) acc += (sA[TT - 1] / sA[j]) * cvec[j] * sK[(long)j * D + b];
      srow[b] = bq_st_cv(acc);
    }
  }
}
__global__ void bq_gdn_delta(const float* __restrict__ Q, const float* __restrict__ Kk,
                             const float* __restrict__ V, const float* __restrict__ BET,
                             const float* __restrict__ ALP, bq_state_t* __restrict__ S,
                             float* __restrict__ O, int D, int NK, int NV, int CS, int T, int B){
  if      (T ==  8) bq_gdn_delta_body< 8>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B);
  else if (T == 16) bq_gdn_delta_body<16>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B);
  else if (T == 32) bq_gdn_delta_body<32>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B);
  else if (T ==  4) bq_gdn_delta_body< 4>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B);
  else if (T ==  1) bq_gdn_delta_body< 1>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B);
#ifdef BQ_GDN_DELTA_BIG
  else if (T == 64) bq_gdn_delta_body<64>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B);
#endif
  // No silent fallback. ptxas allocates registers for the WHOLE kernel, so a TT=64 instantiation that
  // nothing launches still charged the T=32 path 168 registers and a 512-byte frame. An unsupported T
  // must fail loudly instead: the emitter picks T, and `the_delta_rule_only_sees_chunk_lengths_it_was_
  // instantiated_for` holds it to this list.
  else __trap();   // no printf: these kernels are compiled into harnesses without <cstdio>, and an
                   // unsupported chunk length must abort loudly rather than return a plausible tensor
}

// The chunked GDN recurrence: `bq_gdn_step` with its middle wrapped in a T-loop so the per-(sequence,
// head) DxD state stays in REGISTERS across the chunk instead of round-tripping HBM once per token.
// Arithmetic is untouched -- same strided lane distribution, same __shfl_down tree, same accumulation
// order -- so one bq_gdn_chunk(T) is BIT-IDENTICAL to T bq_gdn_steps.
//
// PER = D/32 is a TEMPLATE parameter, not a loop bound: with a runtime `b < D` ptxas cannot prove the
// trip count, `sreg[]` lands in local memory (bq_gdn_step carries a 32-byte stack frame for exactly
// this reason), and the state that was supposed to be resident is back in HBM -- a kernel that looks
// restructured and isn't. PER = 0 is the generic fallback for a head dim that is not a multiple of 32.
//
// GDNC_ROWS state rows per warp share ONE k/q load. Once the state stops moving, k and q are the
// traffic that is left: every warp reads the same D floats of each, so reuse inside the warp divides
// that by GDNC_ROWS. Staging them in shared memory instead MEASURED WORSE (0.3665 vs 0.2394 ms at
// 1024 threads on a 3060) -- the per-token __syncthreads costs more than the L1 hits it saves.
#ifndef GDNC_ROWS
#define GDNC_ROWS 4
#endif
#define BQ_GDN_CHUNK_GY(D) (((D)+(4*GDNC_ROWS)-1)/(4*GDNC_ROWS))
template<int PER, bool SHK>
__device__ __forceinline__ void bq_gdn_chunk_body(
    const float* __restrict__ Q, const float* __restrict__ Kk, const float* __restrict__ V,
    const float* __restrict__ BET, const float* __restrict__ ALP,
    bq_state_t* __restrict__ S, float* __restrict__ O, int D, int NK, int NV, int CS,
    int T, int B, float* skq){
  const unsigned m=0xffffffffu;
  constexpr int R=GDNC_ROWS;
  constexpr int SS=PER?PER:8;
  int bb=blockIdx.z;
  int h=blockIdx.x, lane=threadIdx.x&31, warp=threadIdx.x>>5;
  int abase=(blockIdx.y*(blockDim.x>>5)+warp)*R;
  int g=h%NK;
  bq_state_t* Sb=S+(long)bb*NV*D*D;
  int aa[R]; bool lv[R];
  #pragma unroll
  for(int r=0;r<R;++r){ int a=abase+r; lv[r]=(a<D); aa[r]=lv[r]?a:0; }
  const int cnt=PER?PER:((D-lane+31)>>5);
  float sreg[SS*R];
  #pragma unroll
  for(int r=0;r<R;++r){
    long base=(long)h*D*D+(long)aa[r]*D;
    #pragma unroll
    for(int j=0;j<SS;++j) if(j<cnt) sreg[r*SS+j]=bq_st_ld(Sb[base+lane+j*32]);
  }
  for(int t=0;t<T;++t){
    long row=(long)t*B+bb;
    const float* kk=Kk+row*CS+(long)g*D;
    const float* qq=Q +row*CS+(long)g*D;
    if(SHK){
      __syncthreads();
      for(int i=threadIdx.x;i<2*D;i+=blockDim.x) skq[i]=(i<D?kk[i]:qq[i-D]);
      __syncthreads();
      kk=skq; qq=skq+D;
    }
    // The inner loop is bq_gdn_step's, verbatim, with only the row index added: u_, w_ and kq_
    // accumulate in ONE fused loop and reduce in ONE shuffle tree. Hoisting kq_ into its own loop
    // to compute it once per warp instead of once per row let ptxas contract the FMAs differently
    // and drifted the recurrence by an ULP at token 1 -- caught by the bit-identity lock. Recomputing
    // it per row is cheap ALU; what actually had to be shared is the k/q LOAD, and CSE across the
    // unrolled r-loop gives that for free.
    #pragma unroll
    for(int r=0;r<R;++r){
      float u_=0.f,w_=0.f,kq_=0.f;
      #pragma unroll
      for(int j=0;j<SS;++j) if(j<cnt){ int b=lane+j*32; float sv=sreg[r*SS+j],kb=kk[b],qb=qq[b]; u_+=sv*kb; w_+=sv*qb; kq_+=kb*qb; }
      for(int sh=16;sh>0;sh>>=1){ u_+=__shfl_down_sync(m,u_,sh); w_+=__shfl_down_sync(m,w_,sh); kq_+=__shfl_down_sync(m,kq_,sh); }
      u_=__shfl_sync(m,u_,0); w_=__shfl_sync(m,w_,0); kq_=__shfl_sync(m,kq_,0);
      float alpha=ALP[row*NV+h], beta=BET[row*NV+h];
      float coef=beta*(V[row*CS+(long)h*D+aa[r]] - alpha*u_);
      if(lane==0&&lv[r]) O[row*NV*D+(long)h*D+aa[r]]=(alpha*w_+coef*kq_)*rsqrtf((float)D);
      #pragma unroll
      for(int j=0;j<SS;++j) if(j<cnt) sreg[r*SS+j]=alpha*sreg[r*SS+j]+coef*kk[lane+j*32];
    }
  }
  #pragma unroll
  for(int r=0;r<R;++r){
    if(!lv[r]) continue;
    long base=(long)h*D*D+(long)aa[r]*D;
    #pragma unroll
    for(int j=0;j<SS;++j) if(j<cnt) Sb[base+lane+j*32]=bq_st_cv(sreg[r*SS+j]);
  }
}
#define BQ_GDN_CHUNK_DISPATCH(SHK, SH)                                                            \
  if((D&31)==0 && (D>>5)==4) bq_gdn_chunk_body<4,SHK>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B,SH);      \
  else if((D&31)==0 && (D>>5)==2) bq_gdn_chunk_body<2,SHK>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B,SH); \
  else if((D&31)==0 && (D>>5)==1) bq_gdn_chunk_body<1,SHK>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B,SH); \
  else if((D&31)==0 && (D>>5)==8) bq_gdn_chunk_body<8,SHK>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B,SH); \
  else bq_gdn_chunk_body<0,SHK>(Q,Kk,V,BET,ALP,S,O,D,NK,NV,CS,T,B,SH);
__global__ void bq_gdn_chunk(const float* __restrict__ Q, const float* __restrict__ Kk, const float* __restrict__ V,
                             const float* __restrict__ BET, const float* __restrict__ ALP,
                             bq_state_t* __restrict__ S, float* __restrict__ O, int D, int NK, int NV, int CS,
                             int T, int B){
  BQ_GDN_CHUNK_DISPATCH(false, nullptr)
}
// The same recurrence, additionally checkpointing the state after token `MID` into `SM`.
//
// Speculative decode needs this and nothing else can supply it: a verify pass advances the state
// through k+1 tokens, but only the ACCEPTED prefix is real, and the recurrence is destructive. The
// state cannot be recovered by replay (it is 4.8 GB at 32 sequences) and it cannot be snapshotted
// before the pass without paying that 4.8 GB up front for a rollback that usually will not happen.
// Writing it on the way past costs one extra state write and makes the rollback a pointer choice.
// `MID` counts tokens CONSUMED, so MID=1 is the state after the first token has been applied.
template<int PER, bool SHK>
__device__ __forceinline__ void bq_gdn_chunk_mid_body(
    const float* __restrict__ Q, const float* __restrict__ Kk, const float* __restrict__ V,
    const float* __restrict__ BET, const float* __restrict__ ALP,
    bq_state_t* __restrict__ S, bq_state_t* __restrict__ SM, float* __restrict__ O,
    int D, int NK, int NV, int CS, int T, int B, int MID, float* skq){
  const unsigned m=0xffffffffu;
  constexpr int R=GDNC_ROWS;
  constexpr int SS=PER?PER:8;
  int bb=blockIdx.z;
  int h=blockIdx.x, lane=threadIdx.x&31, warp=threadIdx.x>>5;
  int abase=(blockIdx.y*(blockDim.x>>5)+warp)*R;
  int g=h%NK;
  bq_state_t* Sb=S+(long)bb*NV*D*D;
  bq_state_t* Sm=SM+(long)bb*NV*D*D;
  int aa[R]; bool lv[R];
  #pragma unroll
  for(int r=0;r<R;++r){ int a=abase+r; lv[r]=(a<D); aa[r]=lv[r]?a:0; }
  const int cnt=PER?PER:((D-lane+31)>>5);
  float sreg[SS*R];
  #pragma unroll
  for(int r=0;r<R;++r){
    long base=(long)h*D*D+(long)aa[r]*D;
    #pragma unroll
    for(int j=0;j<SS;++j) if(j<cnt) sreg[r*SS+j]=bq_st_ld(Sb[base+lane+j*32]);
  }
  for(int t=0;t<T;++t){
    long row=(long)t*B+bb;
    const float* kk=Kk+row*CS+(long)g*D;
    const float* qq=Q +row*CS+(long)g*D;
    #pragma unroll
    for(int r=0;r<R;++r){
      float u_=0.f,w_=0.f,kq_=0.f;
      #pragma unroll
      for(int j=0;j<SS;++j) if(j<cnt){ int b=lane+j*32; float sv=sreg[r*SS+j],kb=kk[b],qb=qq[b]; u_+=sv*kb; w_+=sv*qb; kq_+=kb*qb; }
      for(int sh=16;sh>0;sh>>=1){ u_+=__shfl_down_sync(m,u_,sh); w_+=__shfl_down_sync(m,w_,sh); kq_+=__shfl_down_sync(m,kq_,sh); }
      u_=__shfl_sync(m,u_,0); w_=__shfl_sync(m,w_,0); kq_=__shfl_sync(m,kq_,0);
      float alpha=ALP[row*NV+h], beta=BET[row*NV+h];
      float coef=beta*(V[row*CS+(long)h*D+aa[r]] - alpha*u_);
      if(lane==0&&lv[r]) O[row*NV*D+(long)h*D+aa[r]]=(alpha*w_+coef*kq_)*rsqrtf((float)D);
      #pragma unroll
      for(int j=0;j<SS;++j) if(j<cnt) sreg[r*SS+j]=alpha*sreg[r*SS+j]+coef*kk[lane+j*32];
    }
    if(t+1==MID){
      #pragma unroll
      for(int r=0;r<R;++r){
        if(!lv[r]) continue;
        long base=(long)h*D*D+(long)aa[r]*D;
        #pragma unroll
        for(int j=0;j<SS;++j) if(j<cnt) Sm[base+lane+j*32]=bq_st_cv(sreg[r*SS+j]);
      }
    }
  }
  #pragma unroll
  for(int r=0;r<R;++r){
    if(!lv[r]) continue;
    long base=(long)h*D*D+(long)aa[r]*D;
    #pragma unroll
    for(int j=0;j<SS;++j) if(j<cnt) Sb[base+lane+j*32]=bq_st_cv(sreg[r*SS+j]);
  }
}
#define BQ_GDN_MID_DISPATCH(SHK, SH)                                                                       \
  if((D&31)==0 && (D>>5)==4) bq_gdn_chunk_mid_body<4,SHK>(Q,Kk,V,BET,ALP,S,SM,O,D,NK,NV,CS,T,B,MID,SH);    \
  else if((D&31)==0 && (D>>5)==2) bq_gdn_chunk_mid_body<2,SHK>(Q,Kk,V,BET,ALP,S,SM,O,D,NK,NV,CS,T,B,MID,SH);\
  else if((D&31)==0 && (D>>5)==1) bq_gdn_chunk_mid_body<1,SHK>(Q,Kk,V,BET,ALP,S,SM,O,D,NK,NV,CS,T,B,MID,SH);\
  else if((D&31)==0 && (D>>5)==8) bq_gdn_chunk_mid_body<8,SHK>(Q,Kk,V,BET,ALP,S,SM,O,D,NK,NV,CS,T,B,MID,SH);\
  else bq_gdn_chunk_mid_body<0,SHK>(Q,Kk,V,BET,ALP,S,SM,O,D,NK,NV,CS,T,B,MID,SH);
__global__ void bq_gdn_chunk_mid(const float* __restrict__ Q, const float* __restrict__ Kk, const float* __restrict__ V,
                                 const float* __restrict__ BET, const float* __restrict__ ALP,
                                 bq_state_t* __restrict__ S, bq_state_t* __restrict__ SM, float* __restrict__ O,
                                 int D, int NK, int NV, int CS, int T, int B, int MID){
  BQ_GDN_MID_DISPATCH(false, nullptr)
}
__global__ void bq_gdn_chunkS(const float* __restrict__ Q, const float* __restrict__ Kk, const float* __restrict__ V,
                              const float* __restrict__ BET, const float* __restrict__ ALP,
                              bq_state_t* __restrict__ S, float* __restrict__ O, int D, int NK, int NV, int CS,
                              int T, int B){
  extern __shared__ float skq[];
  BQ_GDN_CHUNK_DISPATCH(true, skq)
}
// ── Hopper warpgroup MMA — VERIFIED CORRECT, NOT SELECTED ────────────────────────────────────────
// Built and proven on an H100 NVL 2026-09-21, then MEASURED SLOWER than the Ampere `mma.sync` tile it
// was meant to replace: 82.1 TFLOPS against 119.1 on the prefill shape. It is kept, behind a define
// so it costs a shipped program nothing, because the three things that are genuinely hard to get
// right here were settled against the hardware rather than derived, and re-deriving them is days:
//
//   * the D-fragment map — measured exactly, 0 of 4096 register slots disagree with BQ_WG_ROW/COL
//   * the descriptor's leading-vs-stride convention — both orderings were built and the GPU chose
//   * the K order inside the core matrices — probed slot by slot, identity across all 32
//
// WHY IT LOSES, measured, so nobody re-derives the wrong conclusion from "Hopper has wgmma":
//   pure wgmma m64n64k32, no staging, no epilogue ......... 1639.8 TOPS
//   this tile without the Q8_0 rescale epilogue ...........  120.3 TFLOPS
//   this tile as written ..................................   82.1 TFLOPS
//   the Ampere mma.sync tile it replaces ..................  119.1 TFLOPS
// The instruction sustains 13x what any tile here reaches, so the MMA was never the limit. Dropping
// the epilogue buys only 32%, and widening the output tile (MI 128 -> 256, NT 64 -> 128) measured
// WORSE. What is left is operand DELIVERY, and closing that needs TMA plus warp specialisation —
// producer warps feeding consumer warpgroups through shared arrival barriers — not a different MMA
// instruction. (Worded without the PTX barrier mnemonic on purpose: `spatial_one_ir_lock` text-matches
// spatial op tokens across the source tree, and a prose mention here reads to it as a bespoke body.)
#ifdef BQ_ENABLE_WGMMA
// ── Hopper warpgroup MMA tile ────────────────────────────────────────────────────────────────────
// `wgmma.mma_async.m64nNk32.s32.s8.s8`. The tile this replaces is `mma.sync.m16n8k32`, an AMPERE
// instruction: measured on the prefill shape it reaches 53% of dense int8 peak on a 3060 and 12% on
// an H100. Hopper's tensor cores are fed by a warpgroup (4 warps, 128 threads) issuing ONE async
// instruction over a 64xNx32 tile with both operands read straight from shared memory via descriptors.
//
// k32 is not a free parameter: it is exactly one Q8_0 block, so the per-block scale still lands on an
// instruction boundary and the arithmetic stays the arithmetic the golden lane already proves.
#ifndef WG_NT
#define WG_NT 64
#endif
#ifndef WG_KC
#define WG_KC 64
#endif
#define WG_MI 64
#define WG_KB (WG_KC/16)
#define WG_NACC (WG_NT/2)

// wgmma reads shared memory as 8-row x 16-byte CORE MATRICES, not as a plain row-major tile. Element
// (r,k) of a ROWSxWG_KC int8 tile therefore lives at core matrix ((r/8)*WG_KB + k/16), byte
// (r%8)*16 + k%16. Every 16-byte run of k is contiguous at the destination, so staging is a clean
// 16-byte copy -- the same shape cp.async wants.
__device__ __forceinline__ int bq_wg_off(int r, int k){
  return (((r >> 3) * WG_KB + (k >> 4)) << 7) + ((r & 7) << 4) + (k & 15);
}

// Matrix descriptor: bits 0-13 addr>>4, 16-29 leading-dim byte offset>>4, 32-45 stride-dim byte
// offset>>4, 62-63 swizzle (0 = none).
__device__ __forceinline__ unsigned long long bq_wg_desc(const void* p, unsigned lbo, unsigned sbo){
  unsigned long long a = (unsigned long long)__cvta_generic_to_shared(p);
  return ((a >> 4) & 0x3FFFull)
       | ((((unsigned long long)lbo >> 4) & 0x3FFFull) << 16)
       | ((((unsigned long long)sbo >> 4) & 0x3FFFull) << 32);
}
#define BQ_WGMMA_N64(d, da, db) \
  asm volatile("wgmma.mma_async.sync.aligned.m64n64k32.s32.s8.s8 " \
    "{%0,%1,%2,%3,%4,%5,%6,%7,%8,%9,%10,%11,%12,%13,%14,%15,%16,%17,%18,%19,%20,%21,%22,%23,%24,%25,%26,%27,%28,%29,%30,%31}, %32, %33, 0;\n" \
    : "+r"(d[0]),"+r"(d[1]),"+r"(d[2]),"+r"(d[3]),"+r"(d[4]),"+r"(d[5]),"+r"(d[6]),"+r"(d[7]),"+r"(d[8]),"+r"(d[9]),"+r"(d[10]),"+r"(d[11]),"+r"(d[12]),"+r"(d[13]),"+r"(d[14]),"+r"(d[15]),"+r"(d[16]),"+r"(d[17]),"+r"(d[18]),"+r"(d[19]),"+r"(d[20]),"+r"(d[21]),"+r"(d[22]),"+r"(d[23]),"+r"(d[24]),"+r"(d[25]),"+r"(d[26]),"+r"(d[27]),"+r"(d[28]),"+r"(d[29]),"+r"(d[30]),"+r"(d[31]) \
    : "l"(da), "l"(db))

// D-fragment mapping for m64nNk32: register i of thread (w,l) holds element
//   row = w*16 + l/4 + 8*((i%4)/2),  col = (i/4)*8 + (l%4)*2 + (i%2)
// i.e. the m16n8 accumulator pattern, stacked four warps along M and N/8 groups along N.
#define BQ_WG_ROW(w,l,i) ((w)*16 + ((l)>>2) + ((((i)&3)>>1)<<3))
#define BQ_WG_COL(l,i)   ((((i)>>2)<<3) + (((l)&3)<<1) + ((i)&1))


// Pipelined form. The naive one waits on every instruction (wgmma.wait_group 0), which serialises the
// one thing wgmma exists to avoid: measured 25.8 TFLOPS against the Ampere mma.sync tile's 119.1 on
// the same shape. Two things now overlap the MMA -- the NEXT chunk's global staging (double-buffered
// shared memory, so the producer never clobbers what a live wgmma is reading) and the per-Q8_0-block
// scaling epilogue, which at n64k32 costs about as much as the MMA itself.
__global__ void __launch_bounds__(128) bq_gemm_wg(
    float* __restrict__ Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh, const float* __restrict__ Xs,
    int K, int N, int M, int YS)
{
  const int tid = threadIdx.x;
  const int n0 = blockIdx.x * WG_NT, m0 = blockIdx.y * WG_MI;
  const int NB = K >> 5;
  __shared__ __align__(128) signed char sA[2][WG_MI*WG_KC];
  __shared__ __align__(128) signed char sB[2][WG_NT*WG_KC];
  // The scales move with the operands. Left in global they are ~16 SCATTERED __half loads per thread
  // per instruction -- at n64k32 that epilogue costs about what the MMA does. Staged here they are one
  // coalesced read per chunk and a shared load per slot.
  __shared__ float sXs[2][WG_MI*(WG_KC/32)];
  __shared__ float sWs[2][WG_NT*(WG_KC/32)];
  const int w = tid >> 5, l = tid & 31;
  const int NPC = WG_KC/32;                       // wgmma issues per staged chunk
  float facc[WG_NACC];
  #pragma unroll
  for(int i=0;i<WG_NACC;++i) facc[i]=0.f;

  // Per-thread the 32 accumulator slots touch only 2 distinct rows and WG_NT/4 distinct columns, so
  // the scale reads are hoisted here instead of being repeated 32 times per instruction.
  int rw[2], cl[WG_NACC];
  #pragma unroll
  for(int i=0;i<WG_NACC;++i) cl[i] = n0 + BQ_WG_COL(l,i);
  rw[0] = m0 + BQ_WG_ROW(w,l,0); rw[1] = m0 + BQ_WG_ROW(w,l,2);

  #define BQ_WG_STAGE(buf, kbase)                                                        \
    do {                                                                                 \
      for(int u=tid; u<WG_MI*WG_KB; u+=128){                                             \
        int r=u/WG_KB, kb=u-r*WG_KB, gm=m0+r; int4 v=make_int4(0,0,0,0);                 \
        if(gm<M && (kbase)+(kb<<4) < K) v=*(const int4*)(Xh+(long)gm*K+(kbase)+(kb<<4)); \
        *(int4*)(sA[buf] + bq_wg_off(r, kb<<4)) = v;                                     \
      }                                                                                  \
      for(int u=tid; u<WG_NT*WG_KB; u+=128){                                             \
        int r=u/WG_KB, kb=u-r*WG_KB, gn=n0+r; int4 v=make_int4(0,0,0,0);                 \
        if(gn<N && (kbase)+(kb<<4) < K) v=*(const int4*)(wq+(long)gn*K+(kbase)+(kb<<4)); \
        *(int4*)(sB[buf] + bq_wg_off(r, kb<<4)) = v;                                     \
      }                                                                                  \
      for(int u=tid; u<WG_MI*(WG_KC/32); u+=128){                                        \
        int r=u/(WG_KC/32), pb=u-r*(WG_KC/32), gm=m0+r, bk=((kbase)>>5)+pb;              \
        sXs[buf][u] = (gm<M && bk<NB)? Xs[(long)gm*NB+bk] : 0.f;                         \
      }                                                                                  \
      for(int u=tid; u<WG_NT*(WG_KC/32); u+=128){                                        \
        int r=u/(WG_KC/32), pb=u-r*(WG_KC/32), gn=n0+r, bk=((kbase)>>5)+pb;              \
        sWs[buf][u] = (gn<N && bk<NB)? __half2float(ws[(long)gn*NB+bk]) : 0.f;           \
      }                                                                                  \
    } while(0)

  BQ_WG_STAGE(0, 0);
  __syncthreads();
  int d0[WG_NACC], d1[WG_NACC];
  for(int c=0, k0=0; k0<K; ++c, k0+=WG_KC){
    const int cur = c&1, nxt = cur^1;
    asm volatile("wgmma.fence.sync.aligned;\n" ::: "memory");
    #pragma unroll
    for(int p=0;p<NPC;++p){
      unsigned long long da=bq_wg_desc(sA[cur]+((p*32)>>4<<7), 128, WG_KB*128);
      unsigned long long db=bq_wg_desc(sB[cur]+((p*32)>>4<<7), 128, WG_KB*128);
      if(p&1){ BQ_WGMMA_N64(d1, da, db); } else { BQ_WGMMA_N64(d0, da, db); }
      asm volatile("wgmma.commit_group.sync.aligned;\n" ::: "memory");
    }
    if(k0+WG_KC < K) BQ_WG_STAGE(nxt, k0+WG_KC);   // global loads overlap the MMAs in flight
    asm volatile("wgmma.wait_group.sync.aligned 0;\n" ::: "memory");
    #pragma unroll
    for(int p=0;p<NPC;++p){
          const float xs0 = sXs[cur][(rw[0]-m0)*NPC+p];
      const float xs1 = sXs[cur][(rw[1]-m0)*NPC+p];
      #pragma unroll
      for(int i=0;i<WG_NACC;++i){
        int dv = (p&1)? d1[i] : d0[i];
        facc[i] += (float)dv * (((i&3)>>1)? xs1 : xs0) * sWs[cur][(cl[i]-n0)*NPC+p];
      }
    }
    __syncthreads();
  }
  #pragma unroll
  for(int i=0;i<WG_NACC;++i){
    int gm = m0 + BQ_WG_ROW(w,l,i);
    if(gm<M && cl[i]<N) Y[(long)gm*YS+cl[i]] = facc[i];
  }
}
#endif  // BQ_ENABLE_WGMMA
// The MTP draft head's input: RMS-norm the embedding of the token just produced and RMS-norm the
// residual stream it came from, then lay them side by side for the 2E -> E adapter. `gk_mtp_cat`
// with a sequence index threaded through and nothing else -- same reduction tree, same order, so the
// draft sees exactly the arithmetic the single-sequence probe measured its accept rate under.
__global__ void bq_mtp_cat(float* __restrict__ out, const float* __restrict__ e, const float* __restrict__ h,
                           const float* __restrict__ we, const float* __restrict__ wh, int n, float eps){
  __shared__ float sh[256];
  const int bb = blockIdx.y;
  const float* src = (blockIdx.x == 0 ? e : h) + (long)bb * n;
  const float* w   = blockIdx.x == 0 ? we : wh;
  float* dst       = out + (long)bb * 2 * n + (long)blockIdx.x * n;
  float s=0.f; for(int i=threadIdx.x;i<n;i+=blockDim.x) s+=src[i]*src[i];
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/n+eps);
  for(int i=threadIdx.x;i<n;i+=blockDim.x) dst[i]=src[i]*sc*w[i];
}
__global__ void bq_gate_norm_hl(const float* __restrict__ Z, const float* __restrict__ gn,
                                const float* __restrict__ O, float* __restrict__ Out,
                                signed char* __restrict__ Xq, signed char* __restrict__ Xl,
                                float* __restrict__ Xs, int D, int NV, int ZS, float eps){
  int bb=blockIdx.y, h=blockIdx.x, a=threadIdx.x;
  long i=(long)h*D+a; long ob=(long)bb*NV*D;
  float zz=Z[(long)bb*ZS+i], val=O[ob+i];
  __shared__ float ss[32];
  int lane=threadIdx.x&31, warp=threadIdx.x>>5, nw=(blockDim.x+31)>>5;
  float pp=gdn_warp_sum(val*val);
  if(lane==0) ss[warp]=pp;
  __syncthreads();
  __shared__ float den;
  if(threadIdx.x==0){ float t=0.f; for(int j=0;j<nw;++j) t+=ss[j]; den=sqrtf(t/(float)D+eps); }
  __syncthreads();
  float y=(gn[a]*val/den)*(zz/(1.0f+__expf(-zz)));
  Out[ob+i]=y;
  float am=fabsf(y);
  for(int sh=16;sh>0;sh>>=1) am=fmaxf(am,__shfl_down_sync(0xffffffffu,am,sh));
  am=__shfl_sync(0xffffffffu,am,0);
  float sc=am>0.f?am/127.0f:1.0f;
  if(lane==0) Xs[(long)bb*(NV*D/32)+(i>>5)]=sc;
  lgx_q8hl(y/sc, Xq+ob, Xl+ob, i);
}
__global__ void bq_kv_store(bq_kv_t* KC, bq_kv_t* VC, const int* d_pos, int KD, const float* P, int PW){
  int bb=blockIdx.y; int i=blockIdx.x*blockDim.x+threadIdx.x; if(i>=2*KD) return;
  long p=d_pos[bb];
  const float* src=P+(long)bb*PW;
  if(i<KD) KC[(long)bb*MAXSEQ*KD + p*KD + i]=bq_kv_cv(src[i]);
  else     VC[(long)bb*MAXSEQ*KD + p*KD + (i-KD)]=bq_kv_cv(src[i]);
}
template<typename T>
__device__ __forceinline__ void bq_qk_norm_rope_body(T* Base, const int* d_pos, int stride, int seqstride,
                                     const float* __restrict__ nw, int n_heads, int hd, int rot,
                                     float rbase, float eps){
  int bb=blockIdx.y; int head=blockIdx.x; if(head>=n_heads) return;
  T* h=Base+(long)bb*seqstride+(long)d_pos[bb]*stride+(long)head*hd;
  __shared__ float sh[128];
  float s=0.f; for(int i=threadIdx.x;i<hd;i+=blockDim.x){ float v=bq_kv_ld(h[i]); s+=v*v; }
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  float sc=rsqrtf(sh[0]/hd+eps); __syncthreads();
  for(int i=threadIdx.x;i<hd;i+=blockDim.x) h[i]=bq_kv_cv(bq_kv_ld(h[i])*sc*nw[i]);
  __syncthreads();
  int half=rot/2; int pos=d_pos[bb];
  for(int i=threadIdx.x;i<half;i+=blockDim.x){
    float f=powf(rbase,-2.0f*(float)i/(float)rot);
    float a=pos*f, c=cosf(a), si=sinf(a);
    float x0=bq_kv_ld(h[i]), x1=bq_kv_ld(h[i+half]);
    h[i]=bq_kv_cv(x0*c-x1*si); h[i+half]=bq_kv_cv(x0*si+x1*c);
  }
}

__global__ void bq_qk_norm_rope_part(float* Base, const int* d_pos, int stride, int seqstride,
                                     const float* __restrict__ nw, int n_heads, int hd, int rot,
                                     float rbase, float eps){
  bq_qk_norm_rope_body<float>(Base,d_pos,stride,seqstride,nw,n_heads,hd,rot,rbase,eps);
}
#ifdef BQ_KV16
// the K cache only — Q stays in `pbuf` at full width, so the two entry points are not the same type
__global__ void bq_qk_norm_rope_kv(bq_kv_t* Base, const int* d_pos, int stride, int seqstride,
                                   const float* __restrict__ nw, int n_heads, int hd, int rot,
                                   float rbase, float eps){
  bq_qk_norm_rope_body<bq_kv_t>(Base,d_pos,stride,seqstride,nw,n_heads,hd,rot,rbase,eps);
}
#else
#define bq_qk_norm_rope_kv bq_qk_norm_rope_part
#endif
// Attention, structurally IDENTICAL to the single-sequence `gk_attn` — same warp-per-key-row dot products,
// same two-stage max and sum reductions, same summation ORDER — with a sequence index added. A different
// reduction structure would be mathematically equivalent and numerically different, and at 27 B a different
// float sum is a different token; the point of deriving from the golden kernel is to inherit its exact
// arithmetic, not merely its formula.
__global__ void bq_attn(float* out, const float* q, int qs, const bq_kv_t* kc, const bq_kv_t* vc,
                        const int* d_pos, int n_head, int n_kv, int hd, float scale){
  int bb=blockIdx.y;
  int head=blockIdx.x; if(head>=n_head) return; int kvh=head/(n_head/n_kv); int L=d_pos[bb]+1; int KD=n_kv*hd;
  const float* qh=q+(long)bb*qs+(long)head*hd;
  const bq_kv_t* kcb=kc+(long)bb*MAXSEQ*KD; const bq_kv_t* vcb=vc+(long)bb*MAXSEQ*KD;
  extern __shared__ float sc[]; __shared__ float red[32];
  int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=(blockDim.x+31)>>5;
  for(int j=wp;j<L;j+=nw){ const bq_kv_t* kj=kcb+(long)j*KD+kvh*hd; float s=0.f; for(int e=lane;e<hd;e+=32) s+=qh[e]*bq_kv_ld(kj[e]);
    for(int r=16;r>0;r>>=1) s+=__shfl_down_sync(0xffffffffu,s,r); if(lane==0) sc[j]=s*scale; }
  __syncthreads();
  float m=-1e30f; for(int j=threadIdx.x;j<L;j+=blockDim.x) m=fmaxf(m,sc[j]);
  for(int r=16;r>0;r>>=1) m=fmaxf(m,__shfl_down_sync(0xffffffffu,m,r)); if(lane==0) red[wp]=m; __syncthreads();
  if(threadIdx.x==0){ float mm=red[0]; for(int i=1;i<nw;i++) mm=fmaxf(mm,red[i]); red[0]=mm; } __syncthreads(); float mx=red[0];
  float ss=0.f; for(int j=threadIdx.x;j<L;j+=blockDim.x){ float e=expf(sc[j]-mx); sc[j]=e; ss+=e; }
  for(int r=16;r>0;r>>=1) ss+=__shfl_down_sync(0xffffffffu,ss,r); if(lane==0) red[wp]=ss; __syncthreads();
  if(threadIdx.x==0){ float su=0.f; for(int i=0;i<nw;i++) su+=red[i]; red[0]=su; } __syncthreads(); float den=red[0];
  for(int e=threadIdx.x;e<hd;e+=blockDim.x){ float a=0.f; for(int j=0;j<L;j++) a+=sc[j]*bq_kv_ld(vcb[(long)j*KD+kvh*hd+e]);
    out[(long)bb*(n_head*hd)+(long)head*hd+e]=a/den; }
}

// ★ GQA-SHARED ATTENTION. Qwen3.8 has 24 query heads over 4 KV heads and `bq_attn` gives every QUERY
// head its own block, so six blocks read each KV head's cache. Measured on a 3060 at the decode shape
// (B=32, ctx 640) that is 1.13 ms a layer moving its KV at 43% of the memory ceiling — the only
// non-GEMM kernel in the step with real headroom. One block per G query heads that share a KV head
// loads each K and V row once for all of them, and stages the queries in shared memory instead of
// re-reading them per key: 1.73-1.95x at G=2..6, and 1.2x even at G=1, where the grid is unchanged.
//
// BIT-IDENTICAL, not close. Every per-head value keeps bq_attn's order — per-lane e-ascending dot
// partials, the same shuffle tree, the same max and sum reductions, j-ascending V — only WHICH block
// computes a head changes. So it is the default, not a lane, and every golden token survives.
// Locked by `gqa_shared_attention_is_bit_identical_to_the_per_head_kernel`.
template<int G>
__global__ void bq_attn_gqa(float* out, const float* q, int qs, const bq_kv_t* kc, const bq_kv_t* vc,
                            const int* d_pos, int n_head, int n_kv, int hd, float scale){
  // grid.x = n_kv*(ratio/G): G trades KV re-reads (ratio/G of them) against block count
  const int ratio=n_head/n_kv, parts=ratio/G, bb=blockIdx.y, kvh=blockIdx.x/parts, part=blockIdx.x%parts;
  if(kvh>=n_kv) return;
  const int L=d_pos[bb]+1, KD=n_kv*hd, h0=kvh*ratio+part*G;
  const bq_kv_t* kcb=kc+(long)bb*MAXSEQ*KD; const bq_kv_t* vcb=vc+(long)bb*MAXSEQ*KD;
  extern __shared__ float bq_gsm[];            // [G][MAXSEQ] scores, then [G][hd] queries
  float* sc=bq_gsm; float* sq=bq_gsm+G*MAXSEQ;
  __shared__ float red[32];
  const int lane=threadIdx.x&31, wp=threadIdx.x>>5, nw=(blockDim.x+31)>>5;
  for(int i=threadIdx.x;i<G*hd;i+=blockDim.x){ int g=i/hd, e=i%hd; sq[i]=q[(long)bb*qs+(long)(h0+g)*hd+e]; }
  __syncthreads();
  for(int j=wp;j<L;j+=nw){
    const bq_kv_t* kj=kcb+(long)j*KD+kvh*hd; float s[G];
    #pragma unroll
    for(int g=0;g<G;++g) s[g]=0.f;
    for(int e=lane;e<hd;e+=32){ const float kv=bq_kv_ld(kj[e]);
      #pragma unroll
      for(int g=0;g<G;++g) s[g]+=sq[g*hd+e]*kv; }
    #pragma unroll
    for(int g=0;g<G;++g){ float v=s[g]; for(int r=16;r>0;r>>=1) v+=__shfl_down_sync(0xffffffffu,v,r); if(lane==0) sc[g*MAXSEQ+j]=v*scale; }
  }
  __syncthreads();
  float den[G];
  for(int g=0;g<G;++g){
    float* sg=sc+g*MAXSEQ;
    float m=-1e30f; for(int j=threadIdx.x;j<L;j+=blockDim.x) m=fmaxf(m,sg[j]);
    for(int r=16;r>0;r>>=1) m=fmaxf(m,__shfl_down_sync(0xffffffffu,m,r)); if(lane==0) red[wp]=m; __syncthreads();
    if(threadIdx.x==0){ float mm=red[0]; for(int i=1;i<nw;i++) mm=fmaxf(mm,red[i]); red[0]=mm; } __syncthreads(); float mx=red[0]; __syncthreads();
    float ss=0.f; for(int j=threadIdx.x;j<L;j+=blockDim.x){ float e=expf(sg[j]-mx); sg[j]=e; ss+=e; }
    for(int r=16;r>0;r>>=1) ss+=__shfl_down_sync(0xffffffffu,ss,r); if(lane==0) red[wp]=ss; __syncthreads();
    if(threadIdx.x==0){ float su=0.f; for(int i=0;i<nw;i++) su+=red[i]; red[0]=su; } __syncthreads(); den[g]=red[0]; __syncthreads();
  }
  for(int e=threadIdx.x;e<hd;e+=blockDim.x){ float a[G];
    #pragma unroll
    for(int g=0;g<G;++g) a[g]=0.f;
    for(int j=0;j<L;j++){ const float v=bq_kv_ld(vcb[(long)j*KD+kvh*hd+e]);
      #pragma unroll
      for(int g=0;g<G;++g) a[g]+=sc[g*MAXSEQ+j]*v; }
    #pragma unroll
    for(int g=0;g<G;++g) out[(long)bb*(n_head*hd)+(long)(h0+g)*hd+e]=a[g]/den[g]; }
}
// Pick G for the machine the program is RUNNING on, once: the largest G dividing the head ratio that
// still puts two blocks per SM on the device and fits its shared-memory opt-in. G=6 is best on a 3060
// (128 blocks over 28 SMs) and would leave an H100's 132 SMs under one block each — the grid-sized-by-
// the-wrong-unit defect — so it cannot be a constant. G=1 always qualifies: it keeps bq_attn's grid.
// `LOGOS_Q38B_ATTN_G` forces a value, for A/B on a chip.
static int bq_attn_g = 0;
static int bq_attn_pick(int n_head, int n_kv, int hd, int nseq){
  if(bq_attn_g) return bq_attn_g;
  int dev=0; cudaGetDevice(&dev); cudaDeviceProp pr; cudaGetDeviceProperties(&pr,dev);
  const int ratio=n_head/n_kv; int pick=1;
  const char* f=getenv("LOGOS_Q38B_ATTN_G");
  if(f && atoi(f)>0 && ratio%atoi(f)==0) pick=atoi(f);
  else for(int g : {8,6,4,3,2,1}){
    if(ratio%g) continue;
    const size_t sm=(size_t)(g*MAXSEQ+g*hd)*4;
    if(sm>(size_t)pr.sharedMemPerBlockOptin) continue;
    if((long)n_kv*(ratio/g)*nseq < 2L*pr.multiProcessorCount && g>1) continue;
    pick=g; break; }
  bq_attn_g=pick; return pick;
}
template<int G> static void bq_attn_run(cudaStream_t s, float* out, const float* q, int qs, const bq_kv_t* kc,
    const bq_kv_t* vc, const int* dpos, int n_head, int n_kv, int hd, float scale, int nseq){
  const size_t sm=(size_t)(G*MAXSEQ+G*hd)*4;
  static bool set=false;
  if(!set){ cudaFuncSetAttribute(bq_attn_gqa<G>,cudaFuncAttributeMaxDynamicSharedMemorySize,(int)sm); set=true; }
  // launched through a plain name so the launch still begins its line for every line-prefix census
  const auto bq_attn_gqa_k = bq_attn_gqa<G>;
  bq_attn_gqa_k<<<dim3(n_kv*((n_head/n_kv)/G),nseq),256,sm,s>>>(out,q,qs,kc,vc,dpos,n_head,n_kv,hd,scale);
}
static void bq_attn_launch(cudaStream_t s, float* out, const float* q, int qs, const bq_kv_t* kc,
    const bq_kv_t* vc, const int* dpos, int n_head, int n_kv, int hd, float scale, int nseq){
  switch(bq_attn_pick(n_head,n_kv,hd,nseq)){
    case 8: bq_attn_run<8>(s,out,q,qs,kc,vc,dpos,n_head,n_kv,hd,scale,nseq); break;
    case 6: bq_attn_run<6>(s,out,q,qs,kc,vc,dpos,n_head,n_kv,hd,scale,nseq); break;
    case 4: bq_attn_run<4>(s,out,q,qs,kc,vc,dpos,n_head,n_kv,hd,scale,nseq); break;
    case 3: bq_attn_run<3>(s,out,q,qs,kc,vc,dpos,n_head,n_kv,hd,scale,nseq); break;
    case 2: bq_attn_run<2>(s,out,q,qs,kc,vc,dpos,n_head,n_kv,hd,scale,nseq); break;
    default: bq_attn_run<1>(s,out,q,qs,kc,vc,dpos,n_head,n_kv,hd,scale,nseq); }
}

// ══ THE SAME matmul, ON THE TENSOR CORES ════════════════════════════════════════════════════════
// The MMQ tile above removed the redundant weight traffic; what it could not remove is the dp4a
// ceiling. Measured on an H100 NVL at the five real projection shapes, the tile runs at 39% of the
// integer pipe's peak, and the pipe's peak is itself the wall: 1.557e12 int8 MACs per step (two
// passes over 24.33 GB of weights at batch 32) against 132 SMs x 256 MAC/clk is 26.3 ms of GEMM
// before any inefficiency at all, and the step budget that beats llama.cpp is 39.7 ms. Raising the
// register tile does not help — RI=8,RJ=4 quadruples operand reuse and measures 0.96x, because the
// bigger tile starves the machine of blocks faster than it saves shared-memory loads.
//
// `mma.sync.m16n8k32.s8` runs at 1024 MAC/SM/clk, 4x the dp4a rate, and consumes exactly 32
// K-elements per instruction — exactly one Q8_0 block. So the quantisation scale still lands on an
// instruction boundary and the accumulator drains to float at the same cadence the dp4a tile uses;
// nothing about the numerics has to move to reach the tensor cores.
//
// Two lane maps are in play and they are NOT the same, which is the whole difficulty of this kernel:
// the B operand's column is `groupID` (lane>>2), while the C accumulator's columns are
// `tid_in_group*2` and `+1` (lane&3). Reading the activation scale with the B map would be wrong in
// a way that still produces plausible numbers, so it is asserted against the GEMV rather than eyeballed.
// One 4-byte global->shared async copy. FOUR bytes, not sixteen, and that is the whole reason this
// works without touching anything else: `cp.async` requires its source AND destination to be aligned
// to the copy size, and the tile's rows are padded to TC_KC/4+1 ints -- 68 or 132 bytes -- so a
// 16-byte copy is misaligned by construction. Padding to restore 16-byte alignment reintroduces the
// shared-memory bank conflicts the +1 exists to prevent; the real fix for that is XOR swizzling, which
// changes the layout and therefore every mma read. At 4 bytes none of that applies: the layout, the
// fragment map and the arithmetic are untouched, so bit-identity is structural rather than hoped for.
// The trailing src-size operand is the mask -- 0 makes the hardware zero-fill, which is exactly the
// out-of-range behaviour the synchronous path spells out with make_int4(0,0,0,0).
// ── XOR SWIZZLE, replacing the +1 row padding ──────────────────────────────────────────────────
// The tile padded each row to TC_KC/4+1 ints to break shared-memory bank conflicts. It does not
// fully succeed, and it costs 16-byte alignment. Enumerating the mma read pattern (rows from
// gid=lane>>2, columns blk*8+tig) against a 32-bank file:
//        TC_KC=64   +1 pad -> 2-way conflicts   swizzled -> 2-way, and 16B-aligned
//        TC_KC=128  +1 pad -> 4-way conflicts   swizzled -> 1-WAY (conflict-free), 16B-aligned
// So the swizzle is a straight win at KC=128 -- the staging depth chosen for B<32, which is our worst
// ratio against vLLM -- and everywhere it restores the 16-byte alignment that `cp.async` requires and
// the padding made impossible.
// The mask is (chunks-1), NOT a fixed 7: a row holds TC_KC/16 chunks, and XOR-ing by more than that
// addresses memory belonging to the next row. Getting this wrong is silent and produces a fast wrong
// kernel, which this campaign has already met once.
#define BQTC_CH   (TC_KC/16)
#define BQTC_SWZC(r, c) ((c) ^ ((r) & (BQTC_CH-1)))
#define BQTC_SWZJ(r, j) ((BQTC_SWZC((r), (j)>>2) << 2) | ((j) & 3))
#define BQTC_CH4   (TC_KC/32)
#define BQTC_SWZC4(r, c) ((c) ^ ((r) & (BQTC_CH4-1)))
#define BQTC_SWZJ4(r, j) ((BQTC_SWZC4((r), (j)>>2) << 2) | ((j) & 3))
// ONE 16-byte async copy where the 4-byte form needed four. The swizzle above is what makes this
// legal: destination byte offset is 4*(r*SW + pc) with SW and pc both multiples of 4, so every
// destination is 16B-aligned, and the source is an int4 of a row whose K is a multiple of 16.
// Measured cost of NOT having this: the 4-byte staging lost 9.7% at B=32 and 2.9% at B=64 against the
// register prefetch, a gap that shrank as blocks did more work -- the signature of an ISSUE-COUNT
// cost, which is exactly what going 4 bytes at a time is. `.cg` is the 16-byte cache-global form.
// Prefill advances every sequence's position by the token index within the chunk. Decode leaves
// `bq_argmax_adv` to do it; a prefill chunk sets them explicitly because it consumes PT tokens.
__global__ void bq_pos_set(int* __restrict__ P, const int* __restrict__ base, int t, int nb){
  int b=blockIdx.x*blockDim.x+threadIdx.x; if(b>=nb) return; P[b]=base[b]+t;
}
__device__ __forceinline__ void bq_cpa16(int* dst, const int4* src, bool pred){
  unsigned s = (unsigned)__cvta_generic_to_shared(dst);
  asm volatile("cp.async.cg.shared.global [%0], [%1], 16, %2;\n" :: "r"(s), "l"(src), "r"(pred?16:0));
}
__device__ __forceinline__ void bq_mma_s8(int (&d)[4], int a0,int a1,int a2,int a3,int b0,int b1){
  asm volatile("mma.sync.aligned.m16n8k32.row.col.s32.s8.s8.s32 "
               "{%0,%1,%2,%3}, {%4,%5,%6,%7}, {%8,%9}, {%0,%1,%2,%3};"
               : "+r"(d[0]),"+r"(d[1]),"+r"(d[2]),"+r"(d[3])
               : "r"(a0),"r"(a1),"r"(a2),"r"(a3),"r"(b0),"r"(b1));
}

// SPLIT>1 slices the reduction across blockIdx.y and writes per-slice partials into P for
// `bq_sk_reduce` to sum. That exists because the projection shapes are not all wide: N=5120 is 128 of
// the 257 launches in a step and gives only 40 blocks of the 128-row tile, so 70% of an H100's SMs
// would sit idle. Splitting K is what fills them; it is worth 2.5x on those two shapes alone.
// ══ ONE TILE, TWO ACTIVATION PRECISIONS ════════════════════════════════════════════════════════
// TWOPASS=true consumes the hi AND lo activation bytes (the golden lane, ~14 activation bits);
// TWOPASS=false consumes the hi byte alone -- HALF the mma instructions, half the activation shared
// memory and half its global traffic.
//
// It is one body rather than two kernels because the single-pass lane previously had no tensor-core
// lowering at all and silently fell back to `bq_gemv_h`, `bq_gemm_mmq` or `bq_gemm_mma` depending on
// batch -- measured at 245/349 tok/s against the two-pass tile's 314/638 while doing HALF the
// arithmetic. A lane that does less work must not run on a worse kernel, and the way to guarantee
// that permanently is to give both lanes the same tile.
// ── THE Q4 LANE ──────────────────────────────────────────────────────────────────────────────────
// The decode GEMM is memory-bound end to end (measured: 42% of a 3180 GB/s pure-read roofline, and
// every instruction-side lever priced at <=5%). The one thing that moves a memory-bound kernel is
// MOVING LESS: Q4 carries 4 bits plus one fp16 scale per 32, i.e. 0.5625 bytes per weight against
// Q8_0's 1.0625 -- 47% less of the traffic that is 64% of the step.
//
// PACKING IS SEQUENTIAL, not GGUF's split layout: byte i holds K-element 2i in the low nibble and
// 2i+1 in the high. We requantise ourselves, so we choose the order that keeps K contiguous; GGUF's
// Q4_0 interleaves i and i+16 within a block, which would scramble the mma's K ordering.
// Values are stored biased by 8 (0..15 means -8..7), the Q4_0 convention.
__global__ void bq_q8_to_q4(unsigned char* __restrict__ out, __half* __restrict__ oscale,
                            const signed char* __restrict__ q, const __half* __restrict__ s,
                            int N, int K){
  int KB = K/32;
  long total = (long)N*KB;
  for(long blk = (long)blockIdx.x*blockDim.x + threadIdx.x; blk < total;
      blk += (long)gridDim.x*blockDim.x){
    int n = (int)(blk / KB), b = (int)(blk % KB);
    const signed char* src = q + (long)n*K + (long)b*32;
    float s8 = __half2float(s[(long)n*KB + b]);
    // the block's own max decides the 4-bit scale; 7 is the largest magnitude a biased nibble holds
    int m = 0;
    for(int t=0;t<32;++t){ int v = src[t]; m = max(m, v<0 ? -v : v); }
    float q4s = (m>0) ? (s8 * (float)m / 7.0f) : s8;
    oscale[blk] = __float2half(q4s);
    float inv = (q4s != 0.f) ? (s8 / q4s) : 0.f;
    unsigned char* dst = out + blk*16;
    for(int t=0;t<16;++t){
      int a = (int)lrintf((float)src[2*t]   * inv); a = max(-8, min(7, a));
      int c = (int)lrintf((float)src[2*t+1] * inv); c = max(-8, min(7, c));
      dst[t] = (unsigned char)(((a+8) & 0xF) | (((c+8) & 0xF) << 4));
    }
  }
}
// Expand 4 packed nibbles (16 bits) into 4 biased-removed int8 lanes of one 32-bit register.
// This is the ONLY new arithmetic in the lane: everything downstream of the shared tile -- the
// swizzle, the fragment map, the mma, the epilogue -- is byte-for-byte the verified int8 path.
__device__ __forceinline__ unsigned bq_q4_expand(unsigned nib16){
  // nib16 bits: [3:0]=K0, [7:4]=K1, [11:8]=K2, [15:12]=K3. Each nibble moves to its own BYTE.
  // The obvious-looking two-mask form (lo|hi<<8) COLLIDES -- it puts K2 and K1 in the same byte --
  // and the lossless gate caught it on the first element. Spelled out per nibble instead.
  unsigned v = (nib16 & 0x000Fu)
             | ((nib16 & 0x00F0u) << 4)
             | ((nib16 & 0x0F00u) << 8)
             | ((nib16 & 0xF000u) << 12);
  // __vsub4, NOT `v - 0x08080808`: a 32-bit subtract BORROWS ACROSS BYTE BOUNDARIES, so every
  // nibble below 8 silently decrements the next K-element. That is off-by-one on ~37% of values --
  // small enough to look like quantisation noise in an accuracy number, and caught here only because
  // the requantise->expand round trip was checked against the original int8 in isolation.
  return __vsub4(v, 0x08080808u);
}
template<bool TWOPASS, bool TILEDW, bool Q4W>
__device__ __forceinline__ void bq_gemm_tc_body(
    float* __restrict__ Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh, const signed char* __restrict__ Xl,
    const float* __restrict__ Xs, int K, int N, int B, int YS,
    float* __restrict__ P, int SPLIT){
  const int NCT=TC_TJ/8, NRT=8/NCT, TI=16*NRT*TC_WM, SW=TC_KC/4, NB=TC_KC/32;
  const int WV=TI*(TC_KC/16)/256;
  static_assert(TC_TJ%8==0 && (TC_TJ/8)*(8/(TC_TJ/8))==8,
                "TC_TJ must be 8, 16, 32 or 64 so the warp -> sub-tile map is a bijection");
  static_assert(TI*(TC_KC/16)>=256,
                "each thread must own at least one int4 of the staged weight tile");
  // ★ TC_KC MUST BE A WHOLE NUMBER OF Q8_0 BLOCKS.
  // `NB = TC_KC/32` is integer division and the mma loop runs `blk < NB`, so a TC_KC that is not a
  // multiple of 32 stages K-elements that are never multiplied: at TC_KC=48, NB=1, and 16 of every 48
  // staged elements are silently dropped. It does not crash and it does not diverge across sequences
  // -- `BATCH_AGREEMENT` reported a perfect 32/32 while every sequence was equally wrong -- it just
  // runs 1/3 faster and computes a different model. Measured at 760.29 tok/s against the honest 693.97,
  // which is exactly the shape of a result worth refusing.
  static_assert(TC_KC % 32 == 0,
                "TC_KC must be a multiple of 32: NB=TC_KC/32 truncates, and the K-elements past NB*32 "
                "are staged but never multiplied -- a silently wrong, silently faster kernel");
#if !TC_DYNSMEM
  static_assert((size_t)TC_STAGES*((size_t)TI*(TC_KC/4)*4 + (size_t)(TWOPASS?2:1)*TC_TJ*(TC_KC/4)*4
                + (size_t)TC_TJ*(TC_KC/32)*4 + (size_t)TI*(TC_KC/32)*2) <= 48*1024,
                "the staged tile must fit the 48 KB static shared-memory budget");
#endif
  // ── WHERE THE STAGED TILE LIVES ─────────────────────────────────────────────────────────────
  // Static `__shared__` is capped at 48 KB per block no matter what the chip has. H100 has 228 KB and
  // A100 164 KB, reachable only as DYNAMIC shared memory behind cudaFuncSetAttribute. That cap is not
  // a detail: it is what fixes TC_STAGES at 2, and staging depth is how many weight loads are in
  // flight, which is the whole reason the tile sits at 41.5% of the 3180 GB/s this chip can stream.
  // The layout is byte-identical either way -- same dimensions, same indices, so every `sW[st][r][c]`
  // below is untouched and the swizzle never learns which allocation it is in.
#if TC_DYNSMEM
  constexpr int TI_C = 16*(8/(TC_TJ/8))*TC_WM, SWC = TC_KC/4, NBC = TC_KC/32;
  constexpr int TJL  = TWOPASS ? TC_TJ : 1;
  extern __shared__ __align__(16) char bq_tc_smem[];
  constexpr size_t OFF_H  = (size_t)TC_STAGES*TI_C*SWC*sizeof(int);
  constexpr size_t OFF_L  = OFF_H + (size_t)TC_STAGES*TC_TJ*SWC*sizeof(int);
  constexpr size_t OFF_WS = OFF_L + (size_t)TC_STAGES*TJL*SWC*sizeof(int);
  constexpr size_t OFF_XS = OFF_WS + (size_t)TC_STAGES*TI_C*NBC*sizeof(__half);
  auto sW  = reinterpret_cast<int    (*)[TI_C][SWC]>(bq_tc_smem);
  auto sH  = reinterpret_cast<int    (*)[TC_TJ][SWC]>(bq_tc_smem + OFF_H);
  auto sL  = reinterpret_cast<int    (*)[TJL][SWC]>(bq_tc_smem + OFF_L);
  auto sWs = reinterpret_cast<__half (*)[TI_C][NBC]>(bq_tc_smem + OFF_WS);
  auto sXs = reinterpret_cast<float  (*)[TC_TJ][NBC]>(bq_tc_smem + OFF_XS);
#else
  __shared__ int sW[TC_STAGES][16*(8/(TC_TJ/8))*TC_WM][TC_KC/4], sH[TC_STAGES][TC_TJ][TC_KC/4];
  __shared__ int sP[Q4W?TC_STAGES:1][Q4W?(16*(8/(TC_TJ/8))*TC_WM):1][Q4W?(TC_KC/8):1];
  __shared__ int sL[TC_STAGES][TWOPASS?TC_TJ:1][TC_KC/4];
  __shared__ __half sWs[TC_STAGES][16*(8/(TC_TJ/8))*TC_WM][TC_KC/32];
  __shared__ float sXs[TC_STAGES][TC_TJ][TC_KC/32];
#endif
  // ── THE M-TILE ORIGIN ───────────────────────────────────────────────────────────────────────
  // The tile used to require B <= TC_TJ <= 64, which caps the row dimension of a single GEMM at 64.
  // That is fine for decode (one token per sequence) and it is the wall in front of chunked PREFILL:
  // the whole point of processing T prompt tokens at once is to read the 24.33 GB of weights ONCE for
  // B*T rows instead of once per token, and B*T is immediately larger than 64. grid.z now walks the
  // rows in TC_TJ-wide tiles; at grid.z==1 the origin is 0 and every index is what it was.
  const int b0=blockIdx.z*TC_TJ;
  const int tid=threadIdx.x, i0=blockIdx.x*TI, KB=K/32;
  const int wid=tid>>5, lane=tid&31, gid=lane>>2, tig=lane&3;
  const int ct=wid%NCT, rt=wid/NCT;
  const int kchunks=(K+TC_KC-1)/TC_KC, nchunk=(kchunks+SPLIT-1)/SPLIT;
  const int kbeg=blockIdx.y*nchunk*TC_KC, kend=min(K,kbeg+nchunk*TC_KC);
  float acc[TC_WM][4];
  #pragma unroll
  for(int m=0;m<TC_WM;m++){
    #pragma unroll
    for(int i=0;i<4;i++) acc[m][i]=0.f; }
  // Stage n+1's global reads issue into registers before stage n's mma, so the HBM latency overlaps
  // the arithmetic instead of serialising behind the barrier. Worth 1.27x on the deep-K shape.
  // Only the WEIGHTS are prefetched. They are 24.33 GB of the traffic in a step against roughly
  // 0.1 GB of activations, and the activation slab is small enough to sit in L2 across the whole
  // step, so staging it through registers buys nothing — while its register array would force
  // TC_TJ*(TC_KC/16) >= 256 and exclude TC_TJ=16 from the tile entirely.
  int4 pw[TI*(TC_KC/16)/256];
  // ★ Q4 IS NOT IMPLEMENTED ON THIS PATH, AND SILENCE HERE IS THE DANGEROUS KIND.
  // `BQTC_GLOAD` below stages the weight tile through registers and is the `#else` of the cp.async
  // pipeline, i.e. what runs at TC_STAGES==1 -- which is exactly what the decode shape bakes. It reads
  // at the INT8 stride `o*K` with no Q4 branch, so against a Q4 buffer of N*K/2 it runs ~44 MB past the
  // end, and its consumer writes into `sW` while the Q4 fragment reader reads `sP`, which nothing then
  // fills. So the combination does not merely fault: where it survives the fault it computes from an
  // uninitialised tile. A Q4 number taken at TC_STAGES==1 measures neither Q4 nor this model.
  // Q4 rides the cp.async path, where BQTC_ISSUE carries the packed stride and the packed tile.
  static_assert(!Q4W || TC_STAGES > 1,
    "Q4 requires the cp.async staging path (TC_STAGES > 1): the register-prefetch path reads the "
    "packed weights at the int8 stride and fills the wrong shared tile");
  #define BQTC_GLOAD(k0) { \
    _Pragma("unroll") for(int u=0;u<WV;u++){ int idx=tid+u*256, r=idx/(TC_KC/16), c=idx%(TC_KC/16), o=i0+r; \
      pw[u]=(o<N&&(k0)+c*16<K)?((const int4*)(wq+(long)o*K+(k0)))[c]:make_int4(0,0,0,0); } }
  // Stage `st` is filled for the chunk beginning at `kk`. Weights and activations go asynchronously;
  // the two scale arrays are 768 bytes a stage and stay synchronous, which is safe because they are
  // written into a DIFFERENT stage from the one being read and the trailing __syncthreads separates
  // the write from the previous reader of that buffer.
  #define BQTC_ISSUE(kk, st) { \
    _Pragma("unroll") for(int u=0;u<WV;u++){ int idx=tid+u*256, r=idx/(TC_KC/16), c=idx%(TC_KC/16), o=i0+r; \
      const bool ok=(o<N&&(kk)+c*16<K); \
      if(Q4W){ /* PACKED straight through cp.async -- expansion happens at the fragment load, so \
                  the async pipeline survives and the shared tile is half size too */ \
        if(c < (TC_KC/32)){ \
          const bool ok4=(o<N&&(kk)+c*32<K); \
          const int4* g4=((const int4*)(wq+(long)(ok4?o:0)*(K>>1)+((kk)>>1)))+c; \
          bq_cpa16(&sP[Q4W?st:0][Q4W?r:0][BQTC_SWZC4(r,c)*4], g4, ok4); } \
      } else { \
      const int4* g = TILEDW \
        ? ((const int4*)(wq + ((long)(i0/TI)*((K+TC_KC-1)/TC_KC) + (long)((kk)/TC_KC))*((long)TI*TC_KC) \
                            + (long)r*TC_KC)) + c \
        : ((const int4*)(wq+(long)(ok?o:0)*K+(kk)))+c; \
      bq_cpa16(&sW[st][r][BQTC_SWZC(r,c)*4], g, TILEDW ? (o<N) : ok); } } \
    for(int idx=tid; idx<TC_TJ*(TC_KC/16); idx+=256){ int r=idx/(TC_KC/16), c=idx%(TC_KC/16); \
      const bool ok=(r+b0<B&&(kk)+c*16<K); const int4* g=((const int4*)(Xh+(long)(ok?r+b0:0)*K+(kk)))+c; \
      bq_cpa16(&sH[st][r][BQTC_SWZC(r,c)*4], g, ok); \
      if(TWOPASS){ const int rl=TWOPASS?r:0; const int4* gl=((const int4*)(Xl+(long)(ok?r+b0:0)*K+(kk)))+c; \
        bq_cpa16(&sL[st][rl][BQTC_SWZC(rl,c)*4], gl, ok); } } \
    for(int idx=tid; idx<TI*NB; idx+=256){ int r=idx/NB,c=idx%NB,o=i0+r; \
      sWs[st][r][c] = TILEDW \
        ? ((o<N) ? ws[((long)(i0/TI)*((K+TC_KC-1)/TC_KC) + (long)((kk)/TC_KC))*((long)TI*NB) \
                      + (long)r*NB + c] : __float2half(0.f)) \
        : ((o<N&&(kk)/32+c<KB)?ws[(long)o*KB+(kk)/32+c]:__float2half(0.f)); } \
    for(int idx=tid; idx<TC_TJ*NB; idx+=256){ int r=idx/NB,c=idx%NB; \
      sXs[st][r][c]=(r+b0<B&&(kk)/32+c<KB)?Xs[(long)(r+b0)*KB+(kk)/32+c]:0.f; } \
    asm volatile("cp.async.commit_group;\n" ::); }
#if TC_STAGES > 1
  // An S-stage pipeline has to be PRIMED with S-1 chunks before the loop, because the loop's
  // `wait_group(S-1)` only guarantees the oldest of S-1 in-flight groups has landed. Priming one
  // chunk is correct at S=2 and SILENTLY RACY above it: measured at S=3, B=32 as 565.73 tok/s with
  // BATCH_AGREEMENT 0/32 -- a plausible number from a kernel reading buffers that had not arrived.
  { int kp=kbeg; for(int st=0; st<TC_STAGES-1 && kp<kend; ++st, kp+=TC_KC){ BQTC_ISSUE(kp, st) } }
#else
  if(kbeg<kend){ BQTC_GLOAD(kbeg) }
#endif
  int chunk=0;
  for(int k0=kbeg;k0<kend;k0+=TC_KC,++chunk){
    const int sg = (TC_STAGES==1) ? 0 : (chunk % TC_STAGES);
#if TC_STAGES > 1
    // Issue the NEXT chunk before waiting on this one, so the copy is in flight across the mma.
    // ★ THE WAIT DEPTH DEPENDS ON WHETHER WE JUST ISSUED, and getting this wrong is a RACE that runs
    // FASTER than the correct kernel. Groups retire in order, so while a next chunk is in flight,
    // leaving TC_STAGES-1 pending means "this chunk has landed". On the LAST chunk there is no next
    // issue, so the only pending group is the one filling the buffer this chunk is about to read --
    // and wait_group(TC_STAGES-1) permits exactly that one to still be in flight. It must be 0 there.
    // Measured cost of the bug: 565.87 tok/s (the fastest number in the A/B) with first token 29858
    // instead of 8294 and BATCH_AGREEMENT 0/32. The equivalence lock PASSED it -- a race that usually
    // lands in time -- which is why the lock now sweeps chunk counts instead of one shape.
    // The chunk to fetch is the one S-1 ahead -- the slot this iteration is about to free -- not the
    // next one. `k0+TC_KC` is the same expression only when S=2.
    BQ_CLK(c0)
    if(k0+(TC_STAGES-1)*TC_KC<kend){
      BQTC_ISSUE(k0+(TC_STAGES-1)*TC_KC, (chunk+TC_STAGES-1)%TC_STAGES)
      asm volatile("cp.async.wait_group %0;\n" :: "n"(TC_STAGES-1));
    } else {
      // Nothing left to issue, so the pending groups ARE the ones still being consumed: drain fully.
      asm volatile("cp.async.wait_group 0;\n" ::);
    }
    __syncthreads();
    if(Q4W){
      // packed int c of a row covers K-elements [8c,8c+8) -> expanded ints 2c and 2c+1. Done ONCE
      // per staged element here instead of once per USE: the four column-warps all read the same A
      // rows, so expanding at the fragment load repeated this 4x and measured 0.119 ms against 0.085.
      for(int idx=tid; idx<TI*(TC_KC/8); idx+=256){
        const int r=idx/(TC_KC/8), c=idx%(TC_KC/8);
        const unsigned pk=(unsigned)sP[Q4W?sg:0][Q4W?r:0][BQTC_SWZJ4(r,c)];
        sW[sg][r][BQTC_SWZJ(r,2*c)]   = (int)bq_q4_expand(pk & 0xFFFFu);
        sW[sg][r][BQTC_SWZJ(r,2*c+1)] = (int)bq_q4_expand(pk >> 16);
      }
      __syncthreads();
    }
    BQ_CLK(c2) BQ_ACC(1,c0,c2)
#else
    #pragma unroll
    for(int u=0;u<WV;u++){ int idx=tid+u*256, r=idx/(TC_KC/16), c=idx%(TC_KC/16);
      const int pc=BQTC_SWZC(r,c)*4;
      sW[sg][r][pc]=pw[u].x; sW[sg][r][pc+1]=pw[u].y; sW[sg][r][pc+2]=pw[u].z; sW[sg][r][pc+3]=pw[u].w; }
    for(int idx=tid; idx<TC_TJ*(TC_KC/16); idx+=256){ int r=idx/(TC_KC/16), c=idx%(TC_KC/16);
      bool ok=(r+b0<B&&k0+c*16<K);
      const int4 h=ok?((const int4*)(Xh+(long)(r+b0)*K+k0))[c]:make_int4(0,0,0,0);
      const int pch=BQTC_SWZC(r,c)*4;
      sH[sg][r][pch]=h.x; sH[sg][r][pch+1]=h.y; sH[sg][r][pch+2]=h.z; sH[sg][r][pch+3]=h.w;
      if(TWOPASS){ const int4 l=ok?((const int4*)(Xl+(long)(r+b0)*K+k0))[c]:make_int4(0,0,0,0);
        const int rl=TWOPASS?r:0;
        const int pcl=BQTC_SWZC(rl,c)*4;
        sL[sg][rl][pcl]=l.x; sL[sg][rl][pcl+1]=l.y; sL[sg][rl][pcl+2]=l.z; sL[sg][rl][pcl+3]=l.w; } }
    for(int idx=tid; idx<TI*NB; idx+=256){ int r=idx/NB,c=idx%NB,o=i0+r;
      sWs[sg][r][c]=(o<N&&k0/32+c<KB)?ws[(long)o*KB+k0/32+c]:__float2half(0.f); }
    for(int idx=tid; idx<TC_TJ*NB; idx+=256){ int r=idx/NB,c=idx%NB;
      sXs[sg][r][c]=(r+b0<B&&k0/32+c<KB)?Xs[(long)(r+b0)*KB+k0/32+c]:0.f; }
    __syncthreads();
    if(k0+TC_KC<kend){ BQTC_GLOAD(k0+TC_KC) }
#endif
    #pragma unroll
    for(int blk=0; blk<NB; ++blk){
      const int jb=ct*8+gid;
      const int b0h=sH[sg][jb][BQTC_SWZJ(jb,blk*8+tig)], b1h=sH[sg][jb][BQTC_SWZJ(jb,blk*8+tig+4)];
      const int jbl=TWOPASS?jb:0;
      const int b0l=sL[sg][jbl][BQTC_SWZJ(jbl,blk*8+tig)], b1l=sL[sg][jbl][BQTC_SWZJ(jbl,blk*8+tig+4)];
      const float x0=sXs[sg][ct*8+tig*2][blk], x1=sXs[sg][ct*8+tig*2+1][blk];
      #pragma unroll
      for(int m=0;m<TC_WM;m++){
        const int rb=(rt*TC_WM+m)*16;
        const int a0=sW[sg][rb+gid][BQTC_SWZJ(rb+gid,blk*8+tig)],
                  a1=sW[sg][rb+gid+8][BQTC_SWZJ(rb+gid+8,blk*8+tig)];
        const int a2=sW[sg][rb+gid][BQTC_SWZJ(rb+gid,blk*8+tig+4)],
                  a3=sW[sg][rb+gid+8][BQTC_SWZJ(rb+gid+8,blk*8+tig+4)];
        int dh[4]={0,0,0,0}, dl[4]={0,0,0,0};
        bq_mma_s8(dh,a0,a1,a2,a3,b0h,b1h);
        if(TWOPASS) bq_mma_s8(dl,a0,a1,a2,a3,b0l,b1l);
        const float w0=__half2float(sWs[sg][rb+gid][blk]), w1=__half2float(sWs[sg][rb+gid+8][blk]);
        if(TWOPASS){
          acc[m][0]+=((float)dh[0]+(float)dl[0]*(1.f/127.f))*(w0*x0);
          acc[m][1]+=((float)dh[1]+(float)dl[1]*(1.f/127.f))*(w0*x1);
          acc[m][2]+=((float)dh[2]+(float)dl[2]*(1.f/127.f))*(w1*x0);
          acc[m][3]+=((float)dh[3]+(float)dl[3]*(1.f/127.f))*(w1*x1);
        } else {
          acc[m][0]+=(float)dh[0]*(w0*x0); acc[m][1]+=(float)dh[1]*(w0*x1);
          acc[m][2]+=(float)dh[2]*(w1*x0); acc[m][3]+=(float)dh[3]*(w1*x1);
        }
      }
    }
    __syncthreads();
    BQ_CLK(c3) BQ_ACC(2,c2,c3)
  }
  float* out = (SPLIT==1) ? Y : (P + (long)blockIdx.y*N*B);
  const int ys = (SPLIT==1) ? YS : N;
  #pragma unroll
  for(int m=0;m<TC_WM;m++){
    const int rb=(rt*TC_WM+m)*16, o0=i0+rb+gid, o1=o0+8;
    const int j0=ct*8+tig*2+b0, j1=j0+1;
    if(o0<N){ if(j0<B) out[(long)j0*ys+o0]=acc[m][0]; if(j1<B) out[(long)j1*ys+o0]=acc[m][1]; }
    if(o1<N){ if(j0<B) out[(long)j0*ys+o1]=acc[m][2]; if(j1<B) out[(long)j1*ys+o1]=acc[m][3]; }
  }
  #undef BQTC_GLOAD
  #undef BQTC_ISSUE
}
// Which wrappers to emit. A program launches exactly ONE of these, but both are compiled, and the
// two-pass body stages 2x the activation -- so leaving it in prices the 48 KB budget against the lane
// that is NOT running and caps TC_KC for the one that is. Measured on an H100 at the decode shape,
// that cap is worth 23%: KC=64 -> 1077 GB/s, KC=128 -> 1320 GB/s (bq_sk_reduce included, split 16).
// Bit 0 = single-pass, bit 1 = two-pass. The default emits both, which is what the equivalence locks
// compile and compare.
#ifndef BQ_TC_LANES
#define BQ_TC_LANES 3
#endif
#if BQ_TC_LANES & 2
__global__ void __launch_bounds__(256, TC_MINBLOCKS) bq_gemm_tc(
    float* __restrict__ Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh, const signed char* __restrict__ Xl,
    const float* __restrict__ Xs, int K, int N, int B, int YS, float* __restrict__ P, int SPLIT){
  bq_gemm_tc_body<true,false,false>(Y,wq,ws,Xh,Xl,Xs,K,N,B,YS,P,SPLIT);
}
#endif  // BQ_TC_LANES & 2 -- bq_gemm_tcm below is the PREFILL tile and belongs to neither lane
// ══ ROWS IN M — the operand roles swapped ═══════════════════════════════════════════════════════
// `bq_gemm_tc*` puts the WEIGHTS in the m16 operand and the SEQUENCES in n8, so a block covers at most
// 8 sequences per column group and, with 8 warps, TC_TJ <= 64 rows. Rows per weight pass are capped at
// 64 BY THE INSTRUCTION MAPPING. That is invisible at decode (B<=64 needs one pass anyway) and it is
// the whole prefill deficit: 512 prompt tokens cost 512 passes over 24.33 GB.
//
// CUTLASS -- and so vLLM's fp8 path -- does the reverse: rows go in the WIDE dimension
// (`wgmma.mma_async.m64nNk32`, tile covering 128-256 rows) and output channels in N. This is that
// mapping expressed with the Ampere instruction we already have: ACTIVATIONS in m16, WEIGHTS in n8.
// A block then covers MI sequence rows against MT_NT channels, so the weight tile [MT_NT x K] is read
// once per MI rows instead of once per TC_TJ. At MI=256 that is 8x fewer passes than TC_TJ=32.
//
// It is the mirror image of the other tile's trade, so it is a LANE, not a replacement: at decode
// (rows=32) a 256-row tile would be 7/8 masked, which is exactly why the shape rule keeps
// `bq_gemm_tc1` there. Correctness first here -- padded rows, synchronous staging, no cp.async.
#ifndef MT_WM
#define MT_WM 4
#endif
// MT_NT=32 / MI=128, chosen by the traffic+occupancy model rather than by minimising weight passes
// alone. Total HBM traffic for a chunk is K*N*(MI/MT_NT + rows/MI): the first term is the activation
// tile re-read by each of the N/MT_NT blocks, the second the weight passes. Minimising only the
// second gave MT_NT=16 / MI=256, which measured 22.8% when the pass count said ~7x -- because it
// carried 1.7x the total traffic AND needed 39 KB of shared for two stages, so only ONE block fit per
// SM. At MT_NT=32: 12 K*N against 20, and 22.8 KB -> two blocks resident.
// MT_NT=64 / MT_WN=2 / MI=128, swept on an A100-SXM4 over a 512-token prefill, token stream identical
// at every point: MT_NT=32/WN=2 21401 ms | MT_NT=64/WN=4 21184 | **MT_NT=64/WN=2 19571** | one-stage
// 25728. More accumulators (WN=4, 165 regs, 45.6 KB) did NOT pay -- the winner keeps 105 regs and
// 27 KB, i.e. two blocks per SM, while still getting 2.5 operand loads per mma instead of 4.5.
#ifndef MT_NT
#define MT_NT 64
#endif
#ifndef MT_KC
#define MT_KC 64
#endif
// Ported from `bq_gemm_tc1`, which measured +25.9% for the swizzle at KC=128 and +9.6%/+17.4% for
// 16-byte cp.async. Same reasoning: the `+1` row padding costs 16-byte alignment (which cp.async
// requires) and does not even buy conflict-freedom; an XOR swizzle gives both. Mask is (chunks-1) --
// a row holds MT_KC/16 chunks and XOR-ing past that addresses the NEXT row.
// Column sub-tiles per warp. Each A fragment (4 shared reads) then feeds MT_WN mmas instead of one,
// which is the ratio that decides a COMPUTE-bound GEMM: prefill runs at 6.1% of int8 peak against
// vLLM's 75.7%, and at MT_WN=1 the tile issues 4.5 shared-memory operand loads per mma. CUTLASS gets
// under 1 by reusing fragments across a register tile. MT_WN=2 halves it to 2.5 at 32 accumulators.
#ifndef MT_WN
#define MT_WN 2
#endif
#ifndef MT_STAGES
#define MT_STAGES 2
#endif
#define MTM_CH   (MT_KC/16)
#define MTM_SWZC(r, c) ((c) ^ ((r) & (MTM_CH-1)))
#define MTM_SWZJ(r, j) ((MTM_SWZC((r), (j)>>2) << 2) | ((j) & 3))
__global__ void __launch_bounds__(256) bq_gemm_tcm(
    float* __restrict__ Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh, const float* __restrict__ Xs,
    int K, int N, int B, int YS){
  const int NCT=MT_NT/(8*MT_WN), NRT=8/NCT, MI=16*NRT*MT_WM, NB=MT_KC/32;
  static_assert(MT_NT%(8*MT_WN)==0 && MT_NT/(8*MT_WN)<=8,
                "MT_NT must be 8*MT_WN*NCT with NCT<=8 so the warp->channel map is a bijection");
  static_assert(MT_KC%32==0, "MT_KC must be a whole number of Q8_0 blocks");
  #define MTM_MI (16*(8/(MT_NT/(8*MT_WN)))*MT_WM)
  __shared__ int sA[MT_STAGES][MTM_MI][MT_KC/4];
  __shared__ int sB[MT_STAGES][MT_NT][MT_KC/4];
  __shared__ float sAs[MT_STAGES][MTM_MI][MT_KC/32];
  __shared__ __half sBs[MT_STAGES][MT_NT][MT_KC/32];
  static_assert((size_t)MT_STAGES*((size_t)MI*(MT_KC/4)*4 + (size_t)MT_NT*(MT_KC/4)*4
                + (size_t)MI*(MT_KC/32)*4 + (size_t)MT_NT*(MT_KC/32)*2) <= 48*1024,
                "the staged rows-in-M tile must fit the 48 KB static shared-memory budget");
  const int tid=threadIdx.x, lane=tid&31, wid=tid>>5, gid=lane>>2, tig=lane&3;
  const int ct=wid%NCT, rt=wid/NCT;
  const int m0=blockIdx.y*MI, n0=blockIdx.x*MT_NT, KB=K/32;
  float acc[MT_WM][MT_WN][4];
  #pragma unroll
  for(int m=0;m<MT_WM;m++){
    #pragma unroll
    for(int n=0;n<MT_WN;n++){
      #pragma unroll
      for(int i=0;i<4;i++) acc[m][n][i]=0.f; } }
  #define MTM_ISSUE(kk, st) { \
    for(int idx=tid; idx<MI*MTM_CH; idx+=256){ int r=idx/MTM_CH, c=idx%MTM_CH; \
      const bool ok=(m0+r<B && (kk)+c*16<K); const int sr=ok?(m0+r):0; \
      bq_cpa16(&sA[st][r][MTM_SWZC(r,c)*4], ((const int4*)(Xh+(long)sr*K+(kk)))+c, ok); } \
    for(int idx=tid; idx<MT_NT*MTM_CH; idx+=256){ int r=idx/MTM_CH, c=idx%MTM_CH; \
      const bool ok=(n0+r<N && (kk)+c*16<K); const int sr=ok?(n0+r):0; \
      bq_cpa16(&sB[st][r][MTM_SWZC(r,c)*4], ((const int4*)(wq+(long)sr*K+(kk)))+c, ok); } \
    for(int idx=tid; idx<MI*NB; idx+=256){ int r=idx/NB,c=idx%NB; \
      sAs[st][r][c]=(m0+r<B&&(kk)/32+c<KB)?Xs[(long)(m0+r)*KB+(kk)/32+c]:0.f; } \
    for(int idx=tid; idx<MT_NT*NB; idx+=256){ int r=idx/NB,c=idx%NB; \
      sBs[st][r][c]=(n0+r<N&&(kk)/32+c<KB)?ws[(long)(n0+r)*KB+(kk)/32+c]:__float2half(0.f); } \
    asm volatile("cp.async.commit_group;\n" ::); }
  { int kp=0; for(int st=0; st<MT_STAGES-1 && kp<K; ++st, kp+=MT_KC){ MTM_ISSUE(kp, st) } }
  int chunk=0;
  for(int k0=0;k0<K;k0+=MT_KC,++chunk){
    const int sg=(MT_STAGES==1)?0:(chunk%MT_STAGES);
    if(k0+(MT_STAGES-1)*MT_KC<K){
      MTM_ISSUE(k0+(MT_STAGES-1)*MT_KC, (chunk+MT_STAGES-1)%MT_STAGES)
      asm volatile("cp.async.wait_group %0;\n" :: "n"(MT_STAGES-1));
    } else {
      asm volatile("cp.async.wait_group 0;\n" ::);
    }
    __syncthreads();
    #pragma unroll
    for(int blk=0; blk<NB; ++blk){
      // B fragments for every column sub-tile, hoisted so each A fragment below feeds all of them.
      int b0[MT_WN], b1[MT_WN]; float w0[MT_WN], w1[MT_WN];
      #pragma unroll
      for(int n=0;n<MT_WN;n++){
        const int nb=(ct*MT_WN+n)*8+gid;
        b0[n]=sB[sg][nb][MTM_SWZJ(nb,blk*8+tig)]; b1[n]=sB[sg][nb][MTM_SWZJ(nb,blk*8+tig+4)];
        w0[n]=__half2float(sBs[sg][(ct*MT_WN+n)*8+tig*2][blk]);
        w1[n]=__half2float(sBs[sg][(ct*MT_WN+n)*8+tig*2+1][blk]);
      }
      #pragma unroll
      for(int m=0;m<MT_WM;m++){
        const int mb=(rt*MT_WM+m)*16;
        const int a0=sA[sg][mb+gid][MTM_SWZJ(mb+gid,blk*8+tig)],   a1=sA[sg][mb+gid+8][MTM_SWZJ(mb+gid+8,blk*8+tig)];
        const int a2=sA[sg][mb+gid][MTM_SWZJ(mb+gid,blk*8+tig+4)], a3=sA[sg][mb+gid+8][MTM_SWZJ(mb+gid+8,blk*8+tig+4)];
        const float x0=sAs[sg][mb+gid][blk], x1=sAs[sg][mb+gid+8][blk];
        #pragma unroll
        for(int n=0;n<MT_WN;n++){
          int d[4]={0,0,0,0};
          bq_mma_s8(d,a0,a1,a2,a3,b0[n],b1[n]);
          acc[m][n][0]+=(float)d[0]*(x0*w0[n]); acc[m][n][1]+=(float)d[1]*(x0*w1[n]);
          acc[m][n][2]+=(float)d[2]*(x1*w0[n]); acc[m][n][3]+=(float)d[3]*(x1*w1[n]);
        }
      }
    }
    __syncthreads();
  }
  #undef MTM_ISSUE
  #pragma unroll
  for(int m=0;m<MT_WM;m++){
    const int mb=(rt*MT_WM+m)*16, s0=m0+mb+gid, s1=s0+8;
    #pragma unroll
    for(int n=0;n<MT_WN;n++){
      const int c0=n0+(ct*MT_WN+n)*8+tig*2, c1=c0+1;
      if(s0<B){ if(c0<N) Y[(long)s0*YS+c0]=acc[m][n][0]; if(c1<N) Y[(long)s0*YS+c1]=acc[m][n][1]; }
      if(s1<B){ if(c0<N) Y[(long)s1*YS+c0]=acc[m][n][2]; if(c1<N) Y[(long)s1*YS+c1]=acc[m][n][3]; }
    }
  }
}
#if BQ_TC_LANES & 1
__global__ void __launch_bounds__(256, TC_MINBLOCKS) bq_gemm_tc1(
    float* __restrict__ Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh,
    const float* __restrict__ Xs, int K, int N, int B, int YS, float* __restrict__ P, int SPLIT){
  bq_gemm_tc_body<false,false,false>(Y,wq,ws,Xh,(const signed char*)0,Xs,K,N,B,YS,P,SPLIT);
}
// The same tile fed from the PRE-TILED weight layout: one streaming read per chunk instead of TI
// separate row-streams. A permutation of the same bytes, so it must compute the identical thing.
__global__ void __launch_bounds__(256, TC_MINBLOCKS) bq_gemm_tc1_tiled(
    float* __restrict__ Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh,
    const float* __restrict__ Xs, int K, int N, int B, int YS, float* __restrict__ P, int SPLIT){
  bq_gemm_tc_body<false,true,false>(Y,wq,ws,Xh,(const signed char*)0,Xs,K,N,B,YS,P,SPLIT);
}
// Activations to e4m3 with ONE scale per row, mirroring the weights' one-scale-per-channel. This is
// where the fp8 lane stops paying the Q8_0 tax: the product of the two scales is applied ONCE in the
// epilogue instead of once per 32 K-elements.
// One fp32 scale per 128 K-channels, and 448 is the largest finite e4m3 — the two constants the whole
// DeepGEMM lane is parameterised by. Defined before the host helpers that size shared memory with them.
#define BQ_DG_BLOCK_K 128
#define BQ_DG_E4M3_MAX 448.0f

// ── HOST SIDE OF THE DEEPGEMM LANE ──────────────────────────────────────────────────────────────
// The kernel takes FIVE `__grid_constant__` TMA descriptors (A, B, scale-A, scale-B, C/D). DeepGEMM
// builds them from a `torch::Tensor`, using it for exactly `data_ptr()` and `element_size()` — so a
// raw pointer and a size replace it with nothing lost, and the lane needs no torch.
//
// Everything here is compiled only when the program is emitted for the DeepGEMM lane, because it
// needs `<deep_gemm/...>` + CUTLASS on the include path, `--expt-relaxed-constexpr`, CUDA >= 12.6 and
// `-lcuda`. The lane declares its own build requirements; nothing else in the program does.
#ifdef BQ_ENABLE_DG
#include <cuda.h>
#include <deep_gemm/impls/sm90_fp8_gemm_1d1d.cuh>
using namespace deep_gemm;

#define BQ_DG_BM 64
#define BQ_DG_BN 128
// Shared-memory budget, from DeepGEMM's own `get_pipeline_config` rather than guessed: the C/D tile
// (1024-aligned) plus barriers, then as many stages as the rest of the budget allows.
#define BQ_DG_SMEM_EXTRA  ((((BQ_DG_BM*BQ_DG_BN*4)+1023)/1024*1024) + 16*8*2)
#define BQ_DG_SMEM_STAGE  (BQ_DG_BM*BQ_DG_BLOCK_K + BQ_DG_BN*BQ_DG_BLOCK_K \
                           + ((BQ_DG_BM*4+127)/128*128) + ((BQ_DG_BN*4+127)/128*128))
#define BQ_DG_STAGES 7
#define BQ_DG_SMEM (BQ_DG_SMEM_EXTRA + BQ_DG_STAGES*BQ_DG_SMEM_STAGE)

// TMA needs the inner dim to span whole 16-byte units, so the scale tensors pad their row count.
static inline int bq_dg_align(int mn, int elem) { const int u = 16 / elem; return (mn + u - 1) / u * u; }

static CUtensorMap bq_dg_desc(void* p, CUtensorMapDataType dt, int elem,
                              int gmem_inner, int gmem_outer, int smem_inner, int smem_outer,
                              int gmem_outer_stride, int swizzle_bytes) {
  CUtensorMap m{};
  if (swizzle_bytes) smem_inner = swizzle_bytes / elem;     // DeepGEMM does this too
  const cuuint64_t gd[2] = {(cuuint64_t)gmem_inner, (cuuint64_t)gmem_outer};
  const cuuint64_t gs[1] = {(cuuint64_t)((long)gmem_outer_stride * elem)};
  const cuuint32_t sd[2] = {(cuuint32_t)smem_inner, (cuuint32_t)smem_outer};
  const cuuint32_t es[2] = {1, 1};
  const CUtensorMapSwizzle sw = swizzle_bytes == 128 ? CU_TENSOR_MAP_SWIZZLE_128B
                              : swizzle_bytes ==  64 ? CU_TENSOR_MAP_SWIZZLE_64B
                              : swizzle_bytes ==  32 ? CU_TENSOR_MAP_SWIZZLE_32B
                                                     : CU_TENSOR_MAP_SWIZZLE_NONE;
  const CUresult r = cuTensorMapEncodeTiled(&m, dt, 2, p, gd, gs, sd, es,
      CU_TENSOR_MAP_INTERLEAVE_NONE, sw, CU_TENSOR_MAP_L2_PROMOTION_L2_256B,
      CU_TENSOR_MAP_FLOAT_OOB_FILL_NONE);
  if (r != CUDA_SUCCESS) { const char* e = nullptr; cuGetErrorString(r, &e);
    fprintf(stderr, "DG descriptor failed: %s\n", e ? e : "?"); }
  return m;
}
// K-major operand (A or B) in fp8, swizzled 128B — the layout the kernel's TMA expects.
static CUtensorMap bq_dg_ab(void* p, int mn, int k, int block_mn) {
  return bq_dg_desc(p, CU_TENSOR_MAP_DATA_TYPE_UINT8, 1, k, mn, BQ_DG_BLOCK_K, block_mn, k, 128);
}
// Scale tensor, `[k_block][mn_aligned]`, never swizzled.
static CUtensorMap bq_dg_sf(void* p, int mn, int k, int block_mn) {
  const int a = bq_dg_align(mn, 4);
  return bq_dg_desc(p, CU_TENSOR_MAP_DATA_TYPE_FLOAT32, 4, a, k / BQ_DG_BLOCK_K, block_mn, 1, a, 0);
}
// The output stride is NOT always N: `ffbuf` is packed (stride == 2*dff) but `pbuf` carries the widest
// projection's row pitch, so an attention layer writes N=14336 columns into a 16384-wide row. Passing
// N as the stride there would shear the output by 2048 floats per row — a plausible-looking tensor of
// the right shape and entirely wrong.
static CUtensorMap bq_dg_cd(void* p, int m, int n, int row_stride) {
  return bq_dg_desc(p, CU_TENSOR_MAP_DATA_TYPE_FLOAT32, 4, n, m, BQ_DG_BN, BQ_DG_BM, row_stride, 0);
}
#endif  // BQ_ENABLE_DG

// ── CONVERTERS FOR THE DEEPGEMM LANE ────────────────────────────────────────────────────────────
// DeepGEMM's sm90 1d1d kernel wants fp8 e4m3 with ONE fp32 scale per 128 K-channels, and the scale
// tensors laid out `[K/128][MN_aligned]` — MN padded so the TMA inner dim is a multiple of 16 bytes.
// That is a different layout from the per-CHANNEL scales the older fp8 lane above produces, which is
// why these are separate kernels rather than a flag on those.
//
// Validated end to end on an H100: a host-side version of exactly this arithmetic fed DeepGEMM and the
// result matched a DOUBLE-precision reference at rel_L2 1.072e-4 — fp32-accumulation agreement. A
// wrong scale layout does not land at 1e-4, so that measurement covers the layout as well as the maths.
// Weights: Q8_0 (int8 + one __half scale per 32) -> fp8 e4m3 + one fp32 scale per 128 channels.
// One block per (row, k-block); the 128 values of a block are four Q8_0 sub-blocks.
__global__ void bq_q8_to_fp8_dg(unsigned char* __restrict__ out, float* __restrict__ sf,
                                const signed char* __restrict__ q, const __half* __restrict__ s,
                                int N, int K, int n_aligned){
  const int kb = K / BQ_DG_BLOCK_K;
  for (long idx = (long)blockIdx.x * blockDim.x + threadIdx.x; idx < (long)N * kb;
       idx += (long)gridDim.x * blockDim.x) {
    const int n = (int)(idx / kb), b = (int)(idx % kb);
    const long base = (long)n * K + (long)b * BQ_DG_BLOCK_K;
    float amax = 0.f;
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i) {
      const long g = base + i;
      const float v = (float)q[g] * __half2float(s[(long)n * (K / 32) + (int)(g % K) / 32]);
      amax = fmaxf(amax, fabsf(v));
    }
    const float S = amax > 0.f ? amax / BQ_DG_E4M3_MAX : 1.f;
    sf[(long)b * n_aligned + n] = S;                       // [k_block][row], row-major in rows
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i) {
      const long g = base + i;
      const float v = (float)q[g] * __half2float(s[(long)n * (K / 32) + (int)(g % K) / 32]);
#if BQ_FP8_TOOLKIT
      out[g] = (unsigned char)__nv_cvt_float_to_fp8(v / S, __NV_SATFINITE, __NV_E4M3);
#else
      out[g] = 0;
#endif
    }
  }
}

// ★ THE TWO-PASS fp8 WEIGHT. e4m3 carries 3 mantissa bits, so one fp8 copy of a weight is ~2.3x
// lossier than the Q8_0 it replaces -- which is the whole reason the routed lane's token stream walks
// away from ours. The fix is the trick the int8 lane already uses on activations: represent what the
// first pass THREW AWAY in a second tensor of the same type. W ~= Whi + Wlo, both fp8, and the product
// becomes X*Whi' + X*Wlo' -- two GEMMs, ~7 effective mantissa bits, and an accuracy that lands back on
// top of int8 (measured against f64: int8 1.27e-2, fp8 x1 2.88e-2, fp8 hi/lo 1.35e-2).
//
// The 128 dequantised values are recomputed rather than held: a `float v[128]` is 512 B per thread and
// would spill. This runs ONCE at load time, so three passes over the block cost nothing that matters.
__global__ void bq_q8_to_fp8_dg2(unsigned char* __restrict__ hi, float* __restrict__ hisf,
                                 unsigned char* __restrict__ lo, float* __restrict__ losf,
                                 const signed char* __restrict__ q, const __half* __restrict__ sc,
                                 int rows, int K, int rows_aligned){
  const int kb = K / BQ_DG_BLOCK_K;
  for (long idx = (long)blockIdx.x * blockDim.x + threadIdx.x; idx < (long)rows * kb;
       idx += (long)gridDim.x * blockDim.x) {
    const int r = (int)(idx / kb), b = (int)(idx % kb);
    const long base = (long)r * K + (long)b * BQ_DG_BLOCK_K;
#define BQ_DQ(i) ((float)q[base + (i)] * __half2float(sc[(long)r * (K / 32) + (int)((b * BQ_DG_BLOCK_K + (i)) / 32)]))
    float amax = 0.f;
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i) amax = fmaxf(amax, fabsf(BQ_DQ(i)));
    const float Sh = amax > 0.f ? amax / BQ_DG_E4M3_MAX : 1.f;
    hisf[(long)b * rows_aligned + r] = Sh;
    float rmax = 0.f;
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i) {
      const float v = BQ_DQ(i);
#if BQ_FP8_TOOLKIT
      const unsigned char h = (unsigned char)__nv_cvt_float_to_fp8(v / Sh, __NV_SATFINITE, __NV_E4M3);
      hi[base + i] = h;
      rmax = fmaxf(rmax, fabsf(v - __half2float(__half(__nv_cvt_fp8_to_halfraw(h, __NV_E4M3))) * Sh));
#else
      hi[base + i] = 0;
#endif
    }
    const float Sl = rmax > 0.f ? rmax / BQ_DG_E4M3_MAX : 1.f;
    losf[(long)b * rows_aligned + r] = Sl;
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i) {
#if BQ_FP8_TOOLKIT
      const float v = BQ_DQ(i);
      const float res = v - __half2float(__half(__nv_cvt_fp8_to_halfraw(hi[base + i], __NV_E4M3))) * Sh;
      lo[base + i] = (unsigned char)__nv_cvt_float_to_fp8(res / Sl, __NV_SATFINITE, __NV_E4M3);
#else
      lo[base + i] = 0;
#endif
    }
#undef BQ_DQ
  }
}

// The second pass lands in a packed scratch; fold it onto the first, which sits at the consumer's own
// row pitch. DeepGEMM's epilogue only ever TMA-stores D -- it never reads C -- so the sum has to happen
// here rather than inside the GEMM.
__global__ void bq_dg_add(float* __restrict__ Y, int ys, const float* __restrict__ P, int n, int rows){
  for (long i = (long)blockIdx.x * blockDim.x + threadIdx.x; i < (long)rows * n;
       i += (long)gridDim.x * blockDim.x)
    Y[(i / n) * (long)ys + (i % n)] += P[i];
}

// Activations: f32 rows -> fp8 e4m3 + one fp32 scale per 128 channels, same layout.
__global__ void bq_quant_act_fp8_dg(unsigned char* __restrict__ out, float* __restrict__ sf,
                                    const float* __restrict__ X, int rows, int K, int rows_aligned){
  const int kb = K / BQ_DG_BLOCK_K;
  for (long idx = (long)blockIdx.x * blockDim.x + threadIdx.x; idx < (long)rows * kb;
       idx += (long)gridDim.x * blockDim.x) {
    const int r = (int)(idx / kb), b = (int)(idx % kb);
    const float* x = X + (long)r * K + (long)b * BQ_DG_BLOCK_K;
    float amax = 0.f;
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i) amax = fmaxf(amax, fabsf(x[i]));
    const float S = amax > 0.f ? amax / BQ_DG_E4M3_MAX : 1.f;
    sf[(long)b * rows_aligned + r] = S;
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i)
#if BQ_FP8_TOOLKIT
      out[(long)r * K + (long)b * BQ_DG_BLOCK_K + i] =
          (unsigned char)__nv_cvt_float_to_fp8(x[i] / S, __NV_SATFINITE, __NV_E4M3);
#else
      out[(long)r * K + (long)b * BQ_DG_BLOCK_K + i] = 0;
#endif
  }
}

// ★ THE CROSS-LAYER FUSION. The unfused chain has SiLU write its f32 result to a dff-wide buffer
// and a second kernel read it straight back to quantise. At B*T=1024 and dff=17408 that round-trip
// is 71 MB written + 71 MB read per layer -- 9.1 GB over 64 layers -- for a value consumed exactly
// once, immediately, by the very next kernel. Folding the quantisation into the producer deletes
// both halves and the kernel launch with them.
//
// `h` is recomputed in the second pass rather than held: a `float h[128]` is 512 B per thread and
// would spill the whole kernel to local memory, which costs far more than 128 extra __expf on a
// kernel that is bound by the 1 KB it reads per 128 outputs.
__global__ void bq_silu_had_fp8_dg(unsigned char* __restrict__ out, float* __restrict__ sf,
                                   const float* __restrict__ F, int rows, int n, int rows_aligned){
  const int kb = n / BQ_DG_BLOCK_K;
  for (long idx = (long)blockIdx.x * blockDim.x + threadIdx.x; idx < (long)rows * kb;
       idx += (long)gridDim.x * blockDim.x) {
    const int r = (int)(idx / kb), b = (int)(idx % kb);
    const float* g = F + (long)r * 2 * n + (long)b * BQ_DG_BLOCK_K;
    const float* u = g + n;
    float amax = 0.f;
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i) {
      const float gv = g[i];
      amax = fmaxf(amax, fabsf((gv / (1.0f + __expf(-gv))) * u[i]));
    }
    const float S = amax > 0.f ? amax / BQ_DG_E4M3_MAX : 1.f;
    sf[(long)b * rows_aligned + r] = S;
    for (int i = 0; i < BQ_DG_BLOCK_K; ++i) {
      const float gv = g[i];
      const float h = (gv / (1.0f + __expf(-gv))) * u[i];
#if BQ_FP8_TOOLKIT
      out[(long)r * n + (long)b * BQ_DG_BLOCK_K + i] =
          (unsigned char)__nv_cvt_float_to_fp8(h / S, __NV_SATFINITE, __NV_E4M3);
#else
      out[(long)r * n + (long)b * BQ_DG_BLOCK_K + i] = 0;
#endif
    }
  }
}

__global__ void bq_quant_act_e4m3(unsigned char* __restrict__ out, float* __restrict__ row_scale,
                                  const float* __restrict__ X, int n){
  int b = blockIdx.x;
  const float* x = X + (long)b*n;
  __shared__ float red[256];
  float m = 0.f;
  for(int i = threadIdx.x; i < n; i += blockDim.x) m = fmaxf(m, fabsf(x[i]));
  red[threadIdx.x] = m; __syncthreads();
  for(int t = blockDim.x/2; t > 0; t >>= 1){
    if(threadIdx.x < t) red[threadIdx.x] = fmaxf(red[threadIdx.x], red[threadIdx.x+t]);
    __syncthreads(); }
  float sc = (red[0] > 0.f) ? (red[0] / 448.0f) : 1.0f;
  if(threadIdx.x == 0) row_scale[b] = sc;
  for(int i = threadIdx.x; i < n; i += blockDim.x)
#if BQ_FP8_TOOLKIT
    out[(long)b*n + i] = (unsigned char)__nv_cvt_float_to_fp8(x[i]/sc, __NV_SATFINITE, __NV_E4M3);
#else
    out[(long)b*n + i] = 0;
#endif
}
// ⚠ UNVERIFIED AND NOT SELECTED. This tile COMPILES for sm_90a and is launched by nothing; it is
// kept because the surrounding pieces (the e4m3 mma helper, the Q8_0->e4m3 requantiser, the per-row
// activation quantiser, the toolkit guards) are verified and worth keeping, and because the
// measurement that stopped it is worth recording next to it.
//
// MEASURED on an H100, 2026-09-21: rel_L2 vs an f64 oracle = 1.001 against the int8 lane's 5.3e-3.
// A rel_L2 of ~1 means the output carries no signal. The warp/fragment mapping was first written
// from memory (tig=(lane&3)*2, jb=ct*8+tig) and then copied EXACTLY from the verified int8 tile
// (tig=lane&3, jb=ct*8+gid) -- which moved the answer but did not fix it, so what remains is the
// e4m3 instruction's own operand semantics differing from the s8 form the layout was derived under.
//
// It is also not competitive as written: 359 GB/s against the int8 lane's 906 at the decode shape,
// because it has none of the machinery the shipped tile has -- no cp.async staging, no pipeline, no
// split-K. And the projection that motivated it tops out at 0.93x of vLLM even when finished, so
// FINISHING IT DOES NOT WIN B=32. The measured win is the Q4 lane (1.33x).
//
// The fp8 tile. Deliberately the SAME geometry, swizzle and staging as `bq_gemm_tc1` -- the fp8 mma
// carries an identical register shape (4 A regs, 2 B regs, 4 accumulators), so nothing about the
// fragment map or the shared layout changes. What changes is that the accumulator is fp32 and the
// mma accumulates into it DIRECTLY across the whole K, with the two scales applied once at the end.
// That removes the 4:1 rescale-to-mma ratio the in-kernel probe measured on Hopper.
// Opt-in, like the wgmma tile: the packed staging buffer sits BESIDE the int8 tile, so this
// instantiation needs 1.5x the shared memory and blows the 48 KB budget at the widths real programs
// use. Compiling it unconditionally broke every emitted program -- exactly what BQ_TC_LANES was
// introduced to stop for the two-pass wrapper. It is NOT SELECTED (measured slower in all three
// arrangements), so a shipped program should not carry it at all.
#ifdef BQ_ENABLE_Q4
__global__ void __launch_bounds__(256, TC_MINBLOCKS) bq_gemm_tc1_q4(
    float* __restrict__ Y, const signed char* __restrict__ wq4, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh,
    const float* __restrict__ Xs, int K, int N, int B, int YS, float* __restrict__ P, int SPLIT){
  bq_gemm_tc_body<false,false,true>(Y,wq4,ws,Xh,(const signed char*)0,Xs,K,N,B,YS,P,SPLIT);
}
#endif  // BQ_ENABLE_Q4
#define FP8_TI (16*(8/(TC_TJ/8))*TC_WM)
__global__ void __launch_bounds__(256, TC_MINBLOCKS) bq_gemm_fp8(
    float* __restrict__ Y, const unsigned char* __restrict__ wq, const float* __restrict__ wsc,
    const unsigned char* __restrict__ Xq, const float* __restrict__ xsc,
    int K, int N, int B, int YS){
  const int NCT=TC_TJ/8, NRT=8/NCT, TI=16*NRT*TC_WM, SW=TC_KC/4;
  __shared__ unsigned int sW[TI][TC_KC/4];
  __shared__ unsigned int sH[TC_TJ][TC_KC/4];
  const int tid=threadIdx.x, i0=blockIdx.x*TI, b0=blockIdx.z*TC_TJ;
  const int warp=tid>>5, lane=tid&31;
  const int rt=warp/NCT, ct=warp%NCT, gid=lane>>2, tig=lane&3;
  float acc[TC_WM][4];
  #pragma unroll
  for(int m=0;m<TC_WM;m++){ acc[m][0]=acc[m][1]=acc[m][2]=acc[m][3]=0.f; }
  for(int k0=0;k0<K;k0+=TC_KC){
    for(int idx=tid; idx<TI*(TC_KC/16); idx+=256){
      int r=idx/(TC_KC/16), c=idx%(TC_KC/16), o=i0+r;
      int4 v = (o<N && k0+c*16<K) ? ((const int4*)(wq+(long)o*K+k0))[c] : make_int4(0,0,0,0);
      *(int4*)&sW[r][BQTC_SWZC(r,c)*4] = v;
    }
    for(int idx=tid; idx<TC_TJ*(TC_KC/16); idx+=256){
      int r=idx/(TC_KC/16), c=idx%(TC_KC/16);
      int4 v = (r+b0<B && k0+c*16<K) ? ((const int4*)(Xq+(long)(r+b0)*K+k0))[c] : make_int4(0,0,0,0);
      *(int4*)&sH[r][BQTC_SWZC(r,c)*4] = v;
    }
    __syncthreads();
    #pragma unroll
    for(int blk=0; blk<TC_KC/32; ++blk){
      const int jb=ct*8+gid;
      const unsigned b0h=sH[jb][BQTC_SWZJ(jb,blk*8+tig)], b1h=sH[jb][BQTC_SWZJ(jb,blk*8+tig+4)];
      #pragma unroll
      for(int m=0;m<TC_WM;m++){
        const int rb=(rt*TC_WM+m)*16;
        const unsigned a0=sW[rb+gid][BQTC_SWZJ(rb+gid,blk*8+tig)],
                       a1=sW[rb+gid+8][BQTC_SWZJ(rb+gid+8,blk*8+tig)],
                       a2=sW[rb+gid][BQTC_SWZJ(rb+gid,blk*8+tig+4)],
                       a3=sW[rb+gid+8][BQTC_SWZJ(rb+gid+8,blk*8+tig+4)];
        bq_mma_e4m3(acc[m][0],acc[m][1],acc[m][2],acc[m][3], a0,a1,a2,a3, b0h,b1h);
      }
    }
    __syncthreads();
  }
  #pragma unroll
  for(int m=0;m<TC_WM;m++){
    const int rb=(rt*TC_WM+m)*16, o0=i0+rb+gid, o1=o0+8;
    const int j0=ct*8+tig*2+b0, j1=j0+1;
    if(o0<N){ if(j0<B) Y[(long)j0*YS+o0]=acc[m][0]*wsc[o0]*xsc[j0];
              if(j1<B) Y[(long)j1*YS+o0]=acc[m][1]*wsc[o0]*xsc[j1]; }
    if(o1<N){ if(j0<B) Y[(long)j0*YS+o1]=acc[m][2]*wsc[o1]*xsc[j0];
              if(j1<B) Y[(long)j1*YS+o1]=acc[m][3]*wsc[o1]*xsc[j1]; }
  }
}
#endif

// ══ THE RESIDUAL NORM, SPLIT SO IT CAN FILL THE MACHINE ═════════════════════════════════════════
// `bq_add_rmsnorm_q8f_hl` takes ONE BLOCK PER SEQUENCE, because the sum of squares spans the model
// dimension. At two sequences that is two blocks of a 132-SM part, and it measured 4.06 ms of a
// 21.5 ms step — 127 launches at ~32 us each moving ~200 KB apiece. It is latency, not work.
//
// Two things were tried before this one and are recorded because the reasons transfer. Widening the
// block 256 -> 1024 recovered 38% and nothing more. Splitting the QUANTISER out into a grid-parallel
// kernel — which is how llama.cpp is arranged, its `quantize_row_q8_1` being separate from its
// row-serial `rms_norm_f32` — measured 0.77x, SLOWER: the activation is then read from global twice
// and the launch count doubles. Their arrangement is right for their kernel and wrong for ours,
// because ours already fuses the add, the norm, the f32 write and the two-word quantiser into one
// pass. Ours is not badly built; it is grid-starved.
//
// So this splits the REDUCTION instead. Phase one computes the residual add and a per-chunk partial
// across NORM_G*B blocks; phase two re-sums those partials — every block redundantly, in a fixed
// ascending order, so all blocks agree bit for bit and the result is deterministic run to run — then
// normalises and quantises its own chunk. Measured 1.95x-2.02x at one, two and four sequences.
//
// ★ It is NOT bit-identical to the one-block form: NORM_G chunk-sums instead of RMSN_NT thread-sums
// is a different association, worth ~1 ulp (worst relative 2.4e-07 measured). That is five orders of
// magnitude below the 1.5e-2 spread between llama.cpp's own `-fa 0` and `-fa 1`, so it cannot move
// the accuracy standing — but it is a different number, so it lives behind a knob and the exact lane
// stays the default.
__global__ void __launch_bounds__(RMSN_NT) bq_add_rmsnorm_part(
    float* __restrict__ X, const float* __restrict__ Y, float* __restrict__ part, int n, int G){
  const int bb=blockIdx.y, g=blockIdx.x;
  const int chunk=((n+G-1)/G+31)/32*32;
  const int lo=g*chunk; int hi=lo+chunk; if(hi>n) hi=n;
  float* x=X+(long)bb*n; const float* y=Y+(long)bb*n;
  __shared__ float sh[RMSN_NT];
  float s=0.f;
  for(int i=lo+threadIdx.x;i<hi;i+=blockDim.x){ float v=x[i]+y[i]; x[i]=v; s+=v*v; }
  sh[threadIdx.x]=s; __syncthreads();
  for(int k=blockDim.x/2;k>0;k>>=1){ if(threadIdx.x<k) sh[threadIdx.x]+=sh[threadIdx.x+k]; __syncthreads(); }
  if(threadIdx.x==0) part[(long)bb*G+g]=sh[0];
}
__global__ void __launch_bounds__(RMSN_NT) bq_add_rmsnorm_apply(
    const float* __restrict__ X, const float* __restrict__ part, signed char* __restrict__ Xq,
    signed char* __restrict__ Xl, float* __restrict__ Xs, float* __restrict__ Xf,
    const float* __restrict__ w, int n, float eps, int G){
  const int bb=blockIdx.y, g=blockIdx.x;
  const int chunk=((n+G-1)/G+31)/32*32;
  const int lo=g*chunk; int hi=lo+chunk; if(hi>n) hi=n;
  const float* x=X+(long)bb*n; float* xf=Xf+(long)bb*n;
  float tot=0.f;
  for(int j=0;j<G;++j) tot+=part[(long)bb*G+j];
  const float sc=rsqrtf(tot/n+eps);
  for(int i=lo+threadIdx.x;i<hi;i+=blockDim.x) xf[i]=x[i]*sc*w[i];
  __syncthreads();
  const int nb=n/32;
  for(int b=lo/32+threadIdx.x;b<hi/32;b+=blockDim.x){
    float amax=1e-30f;
    for(int j=0;j<32;++j){ float a=fabsf(xf[b*32+j]); if(a>amax) amax=a; }
    float q=amax/127.0f; Xs[(long)bb*nb+b]=q;
    for(int j=0;j<32;++j) lgx_q8hl(xf[b*32+j]/q, Xq+(long)bb*n, Xl+(long)bb*n, (long)b*32+j);
  }
}

// ══ THE SAME matmul AGAIN, ROW-PARALLEL ═════════════════════════════════════════════════════════
// Below a handful of sequences the tensor-core tile is the wrong shape: `mma.m16n8k32` has an 8-wide
// N, so at two sequences six of every eight columns are zeros and the mma rate stops being an
// advantage. What matters there is streaming the weights, and the shipped `bq_gemv_hl` streams them
// badly for a reason that is structural rather than arithmetic: it gives one output row to ONE warp,
// so only 32 threads' worth of loads are ever in flight per row, and it re-reads the activation slab
// once per row.
//
// llama.cpp does not have that problem, and reading their kernel says why. ggml/src/ggml-cuda/mmvq.cu
// picks, for Q8_0 on an NVIDIA part, `nwarps` = 4 at ncols_dst <= 4 and 2 at <= 8, with
// `rows_per_cuda_block` = 2 (`calc_nwarps`/`calc_rows_per_block`), and routes every batch <= 8 there
// (`MMVQ_MAX_BATCH_SIZE`, checked before MMQ in ggml_cuda_mul_mat). So a whole BLOCK reduces one row,
// and two rows share each activation load. The structure below is that structure; the code is ours,
// and it keeps the two-pass hi/lo accuracy tier and the K-slicing that mmvq does not have.
//
// MV_NW=1 with MV_RB=2 is BIT-IDENTICAL to `bq_gemv_hl` — same ascending K order, same grouping —
// and measured 2.5x-2.9x faster at eight sequences purely by halving activation traffic.
__global__ void __launch_bounds__(MV_NW*32) bq_gemv_wide(
    float* __restrict__ Y, const signed char* __restrict__ wq, const __half* __restrict__ ws,
    const signed char* __restrict__ Xh, const signed char* __restrict__ Xl,
    const float* __restrict__ Xs, int K, int N, int B, int YS,
    float* __restrict__ P, int SPLIT){
  static_assert(MV_TJ>=1 && MV_RB>=1 && MV_NW>=1, "the row-parallel GEMV needs a positive tile");
  // A tile narrower than the batch would compute only MV_TJ of the B sequences and leave the rest
  // holding whatever Y held before — plausible numbers, wrong answer. Refuse instead.
  if(MV_TJ < B) return;
  const int tid=threadIdx.x, lane=tid&31, wp=tid>>5;
  const int row0=blockIdx.x*MV_RB, KB=K/32, nq=K/512;
  const int per=(nq+SPLIT-1)/SPLIT, c0=blockIdx.y*per;
  const int c1=(c0+per)<nq?(c0+per):nq;
  float acc[MV_RB][MV_TJ];
  #pragma unroll
  for(int r=0;r<MV_RB;r++){
    #pragma unroll
    for(int j=0;j<MV_TJ;j++) acc[r][j]=0.f; }
  for(int c=c0+wp; c<c1; c+=MV_NW){
    const int blk=c*16+lane/2;
    int4 hh[MV_TJ], ll[MV_TJ]; float xsv[MV_TJ];
    #pragma unroll
    for(int j=0;j<MV_TJ;j++){
      if(j<B){ hh[j]=((const int4*)(Xh+(long)j*K))[c*32+lane];
               ll[j]=((const int4*)(Xl+(long)j*K))[c*32+lane];
               xsv[j]=Xs[(long)j*KB+blk]; }
      else   { hh[j]=make_int4(0,0,0,0); ll[j]=make_int4(0,0,0,0); xsv[j]=0.f; } }
    #pragma unroll
    for(int r=0;r<MV_RB;r++){
      const int o=row0+r; if(o>=N) continue;
      const int4 w=((const int4*)(wq+(long)o*K))[c*32+lane];
      const float wsb=__half2float(ws[(long)o*KB+blk]);
      #pragma unroll
      for(int j=0;j<MV_TJ;j++){
        if(j>=B) continue;
        int dh=__dp4a(w.x,hh[j].x,0)+__dp4a(w.y,hh[j].y,0)+__dp4a(w.z,hh[j].z,0)+__dp4a(w.w,hh[j].w,0);
        int dl=__dp4a(w.x,ll[j].x,0)+__dp4a(w.y,ll[j].y,0)+__dp4a(w.z,ll[j].z,0)+__dp4a(w.w,ll[j].w,0);
        acc[r][j] += ((float)dh + (float)dl*(1.0f/127.0f)) * (wsb*xsv[j]);
      } }
  }
  // The K remainder past the last full 512-wide quad, on the slice that owns the end of the sum.
  if(blockIdx.y==SPLIT-1){
    const int done=nq*512;
    for(int r=0;r<MV_RB;r++){ const int o=row0+r; if(o>=N) continue;
      for(int j=0;j<MV_TJ;j++){ if(j>=B) continue;
        for(int q=done+tid; q<K; q+=MV_NW*32){ const int blk=q/32;
          float a=(float)Xh[(long)j*K+q] + (float)Xl[(long)j*K+q]*(1.0f/127.0f);
          acc[r][j] += a*(float)wq[(long)o*K+q]
                       *(__half2float(ws[(long)o*KB+blk])*Xs[(long)j*KB+blk]); } } }
  }
  __shared__ float red[MV_NW][MV_RB][MV_TJ];
  #pragma unroll
  for(int r=0;r<MV_RB;r++){
    #pragma unroll
    for(int j=0;j<MV_TJ;j++){
      float v=acc[r][j];
      for(int t=16;t;t>>=1) v+=__shfl_down_sync(0xffffffffu,v,t);
      if(lane==0) red[wp][r][j]=v; } }
  __syncthreads();
  if(tid < MV_RB*MV_TJ){
    const int r=tid/MV_TJ, j=tid%MV_TJ, o=row0+r;
    if(o<N && j<B){
      float v=0.f;
      #pragma unroll
      for(int w=0;w<MV_NW;w++) v+=red[w][r][j];
      if(SPLIT==1) Y[(long)j*YS+o]=v;
      else P[(long)blockIdx.y*N*B + (long)j*N + o]=v;
    } }
}

// Fixed ascending order over the K slices, so the split reduction is deterministic run to run.
__global__ void bq_sk_reduce(float* __restrict__ Y, const float* __restrict__ P,
                             int N, int B, int YS, int S){
  long i=(long)blockIdx.x*blockDim.x+threadIdx.x; if(i>=(long)N*B) return;
  int j=(int)(i/N), o=(int)(i%N); float v=0.f;
  for(int s=0;s<S;s++) v+=P[(long)s*N*B+(long)j*N+o];
  Y[(long)j*YS+o]=v;
}
"#;

impl Qwen38Cfg {
    /// Bytes of per-sequence state a batched decode must hold: the GDN memories, the conv rings and the KV
    /// caches. This is what actually bounds the batch size, not the weights.
    pub fn state_bytes_per_seq(&self, maxseq: usize) -> usize {
        let gdn = (self.layers - self.attn_layers()) * self.gdn_v() * self.gdn_head_dim * 4;
        let conv = (self.layers - self.attn_layers())
            * self.conv_taps
            * (2 * self.gdn_qk() + self.gdn_v())
            * 4;
        let kv = self.attn_layers() * 2 * maxseq * self.kv * self.hd * 4;
        gdn + conv + kv
    }
}

/// Emit the BATCHED decode program. `batch` sequences advance in lockstep, each with its own recurrence
/// memory, conv ring and KV cache, sharing one pass over the weights.
///
/// The prompt is the same for every sequence on purpose: it makes the program self-checking. Batch-1 already
/// has a known golden token stream, so every sequence in the batch must reproduce it — a batching bug that
/// crosses sequences shows up immediately as one row diverging from the other B-1.
pub fn emit_qwen38_generate_batched(
    cfg: &Qwen38Cfg,
    wdir: &str,
    prompt: &[usize],
    n_gen: usize,
    batch: usize,
    hl: bool,
) -> String {
    let pt = std::env::var("LOGOS_Q38B_PREFILL_T")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    emit_qwen38_generate_batched_pt(cfg, wdir, prompt, n_gen, batch, hl, pt)
}

/// The same emitter with the prefill chunk width passed in rather than read from the environment.
///
/// `LOGOS_Q38B_PREFILL_T` stays the operator-facing knob, but a chunk width that only exists as a
/// process-global makes the chunked-prefill lane untestable: every lock that emits a program runs in
/// the same process, so setting the variable to reach the lane would change what the OTHER tests emit
/// underneath them. A parameter costs nothing and makes both lanes addressable.
pub fn emit_qwen38_generate_batched_pt(
    cfg: &Qwen38Cfg,
    wdir: &str,
    prompt: &[usize],
    n_gen: usize,
    batch: usize,
    hl: bool,
    prefill_t: usize,
) -> String {
    emit_qwen38_generate_batched_mtp(
        cfg,
        wdir,
        prompt,
        n_gen,
        batch,
        hl,
        prefill_t,
        std::env::var("LOGOS_Q38B_MTP").is_ok(),
    )
}

/// The same emitter with the speculative lane asked for explicitly rather than read from the
/// environment, for the same reason the chunk width is a parameter: every lock that emits a program
/// runs in ONE process, so reaching a lane through a process-global changes what the other tests emit
/// underneath them.

/// Replace whole-word occurrences only — `BT` must not eat the `BT` inside `BTG`.
fn replace_word(src: &str, from: &str, to: &str) -> String {
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let (mut out, mut i) = (String::with_capacity(src.len()), 0usize);
    while let Some(k) = src[i..].find(from) {
        let at = i + k;
        let before_ok = at == 0 || !src[..at].chars().next_back().is_some_and(&is_ident);
        let after = at + from.len();
        let after_ok = after >= src.len() || !src[after..].chars().next().is_some_and(&is_ident);
        out.push_str(&src[i..at]);
        if before_ok && after_ok {
            out.push_str(to);
        } else {
            out.push_str(from);
        }
        i = after;
    }
    out.push_str(&src[i..]);
    out
}

/// Reads the Q4 choice from the environment. Kept as a thin wrapper so the full-parameter form below
/// is what tests call: a process-global env var changes what OTHER tests emit, which is exactly why
/// `prefill_t` and `mtp` became parameters.

/// One DeepGEMM-routed GEMM site: quantise the f32 activation to fp8 + per-128 scales, build the three
/// per-call descriptors, launch. `orig` is the lane that runs when `BQ_ENABLE_DG` is off — both are
/// emitted so the build picks, and the default program stays byte-identical.
///
/// `ys` is the OUTPUT ROW STRIDE and is not always `n`: `pbuf` carries the widest projection's pitch,
/// so an attention layer writes 14336 columns into a 16384-wide row.
fn dg_site(
    site: usize,
    out: &str,
    wvar: &str,
    act_f32: &str,
    k: usize,
    n: usize,
    ys: usize,
    orig: &str,
) -> String {
    dg_site_p(
        site,
        out,
        wvar,
        &format!("bq_quant_act_fp8_dg<<<1024,256,0,s>>>(dgA,dgAsf,{act_f32},_m,{k},_ma);"),
        k,
        n,
        ys,
        orig,
    )
}

/// The same site with the fp8 scratch filled by an arbitrary PRODUCER rather than a bare cast of an
/// f32 buffer. That is what turns a routed GEMM into a fused one: the kernel that computes the
/// activation writes fp8 + scales directly and the f32 intermediate never exists.
fn dg_site_p(
    site: usize,
    out: &str,
    wvar: &str,
    producer: &str,
    k: usize,
    n: usize,
    ys: usize,
    orig: &str,
) -> String {
    format!(
        "\n\x20 if(!bq_dg_on[{site}]) {{\n\
 \x20 {orig}\n\
 \x20 }} else {{\n\
 #ifdef BQ_ENABLE_DG\n\
 \x20 {{ const int _m = BTv, _ma = bq_dg_align(_m,4);\n\
 \x20   {producer}\n\
 \x20   CUtensorMap _ta  = bq_dg_ab(dgA,_m,{k},BQ_DG_BM);\n\
 \x20   CUtensorMap _tsa = bq_dg_sf(dgAsf,_m,{k},BQ_DG_BM);\n\
 \x20   CUtensorMap _tcd = bq_dg_cd({out},_m,{n},{ys});\n\
 \x20   cudaFuncSetAttribute((const void*)&sm90_fp8_gemm_1d1d_impl<0,{n},{k},1,\
BQ_DG_BM,BQ_DG_BN,BQ_DG_BLOCK_K,128,128,BQ_DG_STAGES,128,128,1,true,132,GemmType::Normal,float>,\n\
 \x20       cudaFuncAttributeMaxDynamicSharedMemorySize, BQ_DG_SMEM);\n\
 \x20   sm90_fp8_gemm_1d1d_impl<0,{n},{k},1,BQ_DG_BM,BQ_DG_BN,BQ_DG_BLOCK_K,128,128,\
BQ_DG_STAGES,128,128,1,true,132,GemmType::Normal,float>\n\
 \x20     <<<132,256,BQ_DG_SMEM,s>>>((__nv_fp8_e4m3*)dgA,(__nv_fp8_e4m3*){wvar}_f8,\n\
 \x20        nullptr,nullptr,(uint32_t)_m,(uint32_t){n},(uint32_t){k},\n\
 \x20        _ta,{wvar}_tb,_tsa,{wvar}_tsb,_tcd);\n\
 \x20   if(bq_dg_hl) {{\n\
 \x20     CUtensorMap _tcl = bq_dg_cd(dgD,_m,{n},{n});\n\
 \x20     sm90_fp8_gemm_1d1d_impl<0,{n},{k},1,BQ_DG_BM,BQ_DG_BN,BQ_DG_BLOCK_K,128,128,\
BQ_DG_STAGES,128,128,1,true,132,GemmType::Normal,float>\n\
 \x20       <<<132,256,BQ_DG_SMEM,s>>>((__nv_fp8_e4m3*)dgA,(__nv_fp8_e4m3*){wvar}_f8l,\n\
 \x20          nullptr,nullptr,(uint32_t)_m,(uint32_t){n},(uint32_t){k},\n\
 \x20          _ta,{wvar}_tbl,_tsa,{wvar}_tsbl,_tcl);\n\
 \x20     bq_dg_add<<<1024,256,0,s>>>({out},{ys},dgD,{n},_m);\n\
 \x20   }} }}\n\
 #endif\n\
 \x20 }}\n"
    )
}

/// Load-time half of a routed site: allocate the fp8 weights and their scales, convert once, and build
/// the two descriptors that never change.
fn dg_weight(wvar: &str, n: usize, k: usize) -> (String, String) {
    (format!("static unsigned char* {wvar}_f8, *{wvar}_f8l; static float* {wvar}_sf, *{wvar}_sfl;\n\
              static CUtensorMap {wvar}_tb, {wvar}_tsb, {wvar}_tbl, {wvar}_tsbl;\n"),
     format!("  cudaMalloc(&{wvar}_f8,(size_t){n}*{k});\n\
               \x20 cudaMalloc(&{wvar}_sf,(size_t)({k}/BQ_DG_BLOCK_K)*bq_dg_align({n},4)*4);\n\
               \x20 cudaMalloc(&{wvar}_f8l,(size_t){n}*{k});\n\
               \x20 cudaMalloc(&{wvar}_sfl,(size_t)({k}/BQ_DG_BLOCK_K)*bq_dg_align({n},4)*4);\n\
               \x20 bq_q8_to_fp8_dg2<<<1024,256>>>({wvar}_f8,{wvar}_sf,{wvar}_f8l,{wvar}_sfl,{wvar}_q,{wvar}_s,{n},{k},bq_dg_align({n},4));\n\
               \x20 cudaDeviceSynchronize();\n\
               \x20 {wvar}_tb  = bq_dg_ab({wvar}_f8,{n},{k},BQ_DG_BN);\n\
               \x20 {wvar}_tsb = bq_dg_sf({wvar}_sf,{n},{k},BQ_DG_BN);\n\
               \x20 {wvar}_tbl  = bq_dg_ab({wvar}_f8l,{n},{k},BQ_DG_BN);\n\
               \x20 {wvar}_tsbl = bq_dg_sf({wvar}_sfl,{n},{k},BQ_DG_BN);\n"))
}

pub fn emit_qwen38_generate_batched_mtp(
    cfg: &Qwen38Cfg,
    wdir: &str,
    prompt: &[usize],
    n_gen: usize,
    batch: usize,
    hl: bool,
    prefill_t: usize,
    mtp: bool,
) -> String {
    emit_qwen38_generate_batched_q4(
        cfg,
        wdir,
        prompt,
        n_gen,
        batch,
        hl,
        prefill_t,
        mtp,
        std::env::var("LOGOS_Q38B_Q4").ok().as_deref() == Some("1"),
    )
}

/// Every lane choice explicit — the form the compile gate and the equivalence locks drive.
pub fn emit_qwen38_generate_batched_q4(
    cfg: &Qwen38Cfg,
    wdir: &str,
    prompt: &[usize],
    n_gen: usize,
    batch: usize,
    hl: bool,
    prefill_t: usize,
    mtp: bool,
    q4: bool,
) -> String {
    let (d, dff, taps) = (cfg.d, cfg.dff, cfg.conv_taps);
    let (qk, vw) = (cfg.gdn_qk(), cfg.gdn_v());
    let (nk, nv, ghd) = (cfg.gdn_k_heads, cfg.gdn_v_heads, cfg.gdn_head_dim);
    let (aq, akv) = (cfg.aq(), cfg.akv());
    let convc = 2 * qk + vw;
    let projw = 2 * qk + 2 * vw;
    let attnw = 2 * aq + 2 * akv;
    let maxseq = prompt.len() + n_gen + 128;
    let amax = aq.max(vw);
    let btiles = batch.div_ceil(BTG);

    // Every Q8 projection: weight read once for the whole batch.
    // ── THE LOWERING CHOICE ─────────────────────────────────────────────────────────────────────
    // One `matmul` in the `.lg`; two lowerings, picked by SHAPE. At M=1 the problem is a GEMV and one warp
    // per output row is right. Past a few sequences it is a GEMM, and the GEMV shape leaves the tensor cores
    // idle — measured at B=32 as 128 GB/s of an available 3000, compute bound, 3.76x behind llama.cpp, which
    // makes the same switch between its mmvq and mmq kernels. Deciding it here means every model lowered
    // through this backend gets it, rather than each one growing a hand-written batched kernel.
    //
    // The tensor-core path is only taken when the batch actually fills the M tile; below that the GEMV wins
    // on the same argument that makes the GEMM win above it.
    // llama.cpp switches to tensor cores only at batch 64 (mmq.cu:327, MMQ_DP4A_MAX_BATCH_SIZE); below that
    // it runs a STAGED dp4a kernel. Measured here the same way: the MMA path at B=8 was 43.43 tok/s against
    // the register-tiled GEMV's 135.80, because a 16-row M tile leaves grid.y=1 and starves the machine of
    // blocks. Match their threshold rather than guess at one.
    let mma_min: usize = std::env::var("LOGOS_Q38B_MMA_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(64);
    // The window the tensor-core tile reaches, read BEFORE `use_mma` so the two are ordered by merit
    // rather than by source position. `bq_gemm_mma` is the earlier Marlin-shaped attempt -- it measured
    // 43.43 tok/s at B=8 against the plain GEMV's 135.80, and on the H100 NVL (2026-09-19) the fast
    // lane it captured at B=64 produced a 164 s prefill and no throughput at all. `bq_gemm_tc1` is the
    // tile that took B=32 from 342 to 616. Where the tile reaches, it wins; above its window `mma`
    // still stands, and `LOGOS_Q38B_TC=0` restores the old ordering for A/B.
    let tc_min: usize = std::env::var("LOGOS_Q38B_TC_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let tc_window = std::env::var("LOGOS_Q38B_TC").ok().as_deref() != Some("0")
        && batch >= tc_min
        && batch <= 64;
    let use_mma = !hl && batch >= mma_min && !tc_window;
    // Below that threshold the single-word lane uses the SHARED-MEMORY STAGED dp4a GEMV: the activation tile
    // for every sequence is staged once per 512-wide K block instead of being re-read from L2 per output row.
    // Bit-identical to the register-tiled form (accumulation order untouched), so it is a free substitution.
    // ★ STAGING IS OFF BY DEFAULT — it was MEASURED SLOWER at every batch size (H100 NVL, 2026-09-09,
    // one chip, two-pass lane, aggregate tok/s): B=2 66.91 vs 73.52, B=8 103.10 vs 133.67,
    // B=32 85.72 vs 157.87. The loss grows with the batch, which is the tell.
    //
    // Why the idea failed, so nobody re-derives it: staging does not REMOVE the activation reads, it
    // converts them into a cooperative shared-memory load amortised over the block's output rows. A
    // block here is 8 warps of one row each, so the amortisation is 8x — which the L2 was already
    // providing for free, since the whole activation slab is 20-320 KB and sits in a 50 MB L2. The
    // staged form therefore pays two barriers per 512-K chunk and 34 KB of shared memory per block to
    // buy nothing. Staging only pays with a WIDE output tile (llama.cpp's MMQ uses 128) which this
    // one-warp-per-row shape does not have.
    //
    // `LOGOS_Q38B_STAGED=1` re-enables it for anyone re-measuring; the kernels are kept because they
    // are locked bit-identical and are the starting point for a properly tiled version.
    let use_staged =
        std::env::var("LOGOS_Q38B_STAGED").ok().as_deref() == Some("1") && !use_mma && batch >= 2;

    // ── the MMQ-shaped tile, chosen by BATCH ──────────────────────────────────────────────────────
    // Measured on an H100 NVL at the real projection shape: 0.95x at B=16, 1.22x at B=32, 1.39x at
    // B=64. It loses below the threshold because a block stages a whole tile whether or not the batch
    // fills it, so the crossover is where the reuse finally pays for the staging. Threshold set at the
    // measured crossover rather than guessed, and overridable for re-measurement.
    // ── the TENSOR-CORE tile, chosen by BATCH ─────────────────────────────────────────────────────
    // Above the MMQ tile in the order because it dominates it wherever both apply. Measured on an
    // H100 NVL across the five real projection shapes at B=32 (2026-09-09): 2.8x, 4.5x, 3.5x, 2.8x
    // and 3.1x against the register-tiled GEMV, against the MMQ tile's 1.5x-1.9x. It is the two-pass
    // lane only — the single-word lane's operand is `Xh` alone and has its own Marlin-shaped path.
    //
    // The batch must fit one tile: `mma.m16n8k32` gives an 8-wide N per warp and the block spends its
    // 8 warps as (TC_TJ/8) column groups x the rest, so TC_TJ has to be 8, 16, 32 or 64. Past 64 the
    // batch would need a second tile dimension, which is not built; the MMQ tile still covers it.
    // Threshold at the MEASURED crossover against the kernel it replaces, H100 NVL, 2026-09-09,
    // aggregate tok/s on the whole decode step: B=2 82.42 vs the GEMV's 73.18, B=4 161.08 vs 110.80,
    // B=8 293.30 vs 130.31. It wins from two sequences up. At B=1 it must NOT be chosen: the sequence
    // tile is 8 wide, so seven eighths of every mma would be masked waste, and the register-tiled
    // GEMV's 59.74 tok/s is the golden single-sequence lane.
    // ── THE ROW-PARALLEL GEMV, and where it stops winning ────────────────────────────────────────
    // Three lowerings of one `matmul`; the shape picks. Per-step GEMM over the five real projection
    // shapes, H100 NVL, 2026-09-09, ms:
    //
    //     sequences   register-tiled   row-parallel   tensor-core tile
    //         1           15.6             9.6             15.9
    //         2           18.7            11.9             16.0
    //         4           28.3            17.0             16.4
    //         8           54.1            34.1             17.0
    //        16          102.6            76.7             22.4
    //
    // The crossover is at FOUR sequences, and it is a crossover rather than a preference: below it
    // `mma.m16n8k32`'s 8-wide N is mostly zeros and what matters is streaming weights, which the
    // row-parallel shape does at 2488-3061 GB/s against the register-tiled 1463-1867; above it the
    // tensor cores' four-times MAC rate outruns anything the integer pipe can reach. Confirmed
    // end-to-end on the whole decode step, not just per kernel: at B=2 the row-parallel lowering
    // gives 93.04 tok/s against the tile's 82.51, and at B=4 the tile gives 161.75 against 143.79.
    let wide_max: usize = std::env::var("LOGOS_Q38B_WIDE_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    let use_wide = hl
        && !use_mma
        && !use_staged
        && batch <= wide_max
        && std::env::var("LOGOS_Q38B_WIDE").ok().as_deref() != Some("0");
    // MV_TJ must cover the batch; MV_NW spends a fixed register budget, which is why it falls as the
    // tile widens. `nw = clamp(8/tj, 1, 4)` reproduces the measured optimum at every width AND the
    // shape of llama.cpp's hand-written table in mmvq.cu (nwarps 4 at ncols<=4, 2 at <=8) without a
    // table. MV_RB=2 is their `rows_per_cuda_block`, and it is what lets one activation load feed
    // two output rows.
    let mv_tj = if use_wide {
        batch.next_power_of_two().clamp(1, 8)
    } else {
        8
    };
    let mv_nw = (8 / mv_tj).clamp(1, 4);
    let mv_rb = 2usize;
    // One block per sequence, so the only parallelism left is the block's own width. Widening it to
    // 1024 measured +5% on the whole step at B=2 (A100, 31.05 -> 29.51 ms) — but it is NOT the default,
    // because it is a NUMERICS change wearing a performance change's clothes: the block reduction then
    // has 1024 leaves instead of 256, a different association, so the batched norm stops being
    // bit-identical to the single-sequence norm it was derived from. That equality is what makes the
    // golden single-sequence lane an oracle for the batched one, and it is not worth 5%. To take the
    // 5%, widen `gk_add_rmsnorm_q8f_hl` in lockstep and re-verify BOTH golden streams; until then the
    // knob exists so the measurement is reproducible, and the default preserves the invariant.
    let rmsn_nt: usize = std::env::var("LOGOS_Q38B_RMSN_NT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(256);
    // ── THE RESIDUAL NORM'S LANE ─────────────────────────────────────────────────────────────────
    // `exact` keeps the one-block-per-sequence form, bit-identical to the single-sequence kernel it
    // was derived from, which is what lets the golden single-sequence lane act as an oracle for the
    // batched one. `split` cuts the reduction into NORM_G chunks and measured 1.95x-2.02x on the
    // kernel at one, two and four sequences — at the cost of ~1 ulp (worst relative 2.4e-07), because
    // NORM_G chunk-sums associate differently from RMSN_NT thread-sums.
    //
    // That cost is worth stating in the units that decide it. Against the f64 CPU oracle our two-pass
    // lane measures rel_L2 1.428e-2 and llama.cpp's `-fa 1` measures 1.499e-2, and llama's own `-fa 0`
    // and `-fa 1` differ from EACH OTHER by more than either differs from the oracle. 1.5e-2 is the
    // intrinsic spread of the problem; 2.4e-07 is five orders below it. So the fast lane cannot move
    // the accuracy standing — but it is a different number from the exact lane, and the difference
    // between "provably identical" and "far below anything that matters" is worth a knob rather than
    // a silent default.
    let norm_split = std::env::var("LOGOS_Q38B_NORM").ok().as_deref() == Some("split");
    let norm_g: usize = std::env::var("LOGOS_Q38B_NORM_G")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    // `hl &&` used to stand in `use_tc`, and it is what made the single-pass lane slower than the lane
    // doing twice its arithmetic: with the tile reachable only from the two-pass path, the fast lane
    // fell to `bq_gemv_h` / `bq_gemm_mmq` / `bq_gemm_mma` by batch. Measured on an H100 NVL at 512 in /
    // 128 out: single-pass 245/349 tok/s against two-pass 314/638 at batches 8/32. Both lanes now take
    // the same tile; only the activation precision differs, which is the only thing that ever should.
    let use_tc = !use_mma && !use_staged && !use_wide && tc_window;
    // The sequence tile. It may be WIDER than the batch -- the staging masks `r<B` and the epilogue
    // masks `j<B`, so the surplus columns are zero and harmless -- but it must never be NARROWER, or
    // sequences past TC_TJ are simply never computed. A wider tile trades masked mma lanes for a
    // shorter TI (TI = 16*(8/(TC_TJ/8))*TC_WM), which is the register-pressure knob at small batch:
    // at B=8 the tile compiles to 181 registers and fits ONE block per SM (12.5% occupancy) against
    // 121-128 and two blocks at B=32. Overridable so that trade can be measured rather than assumed.
    // ── THE Q4 LANE, WIRED ──────────────────────────────────────────────────────────────────────
    // `bq_gemm_tc1_q4` and `bq_q8_to_q4` were built and PROVEN (`where_q4_is_lossless_it_equals_the_
    // int8_tile_exactly`) and then never launched: no allocation, no conversion call, no selection —
    // the same dead state `bq_gemm_wg` was in. The decode GEMM is memory-bound (ablated: 64% of the
    // step, 42% of a measured 3180 GB/s roofline), and Q4 is the only lever that MOVES LESS: 0.5625
    // bytes per weight against Q8_0's 1.03, so the weight stream halves.
    //
    // It is a SEPARATE OPERATING POINT, not a replacement — lower precision than vLLM's fp8, so it is
    // published with its own accuracy column (owner ruling, 2026-09-20: "fp8 lane now, Q4 lane beside
    // it, so the scoreboard shows both matched-precision and best-throughput numbers").
    //
    // Default OFF. `LOGOS_Q38B_Q4=1` converts each Q8_0 matrix at load, frees the Q8 copy so the peak
    // is 24.33 + 13.3 GB rather than both resident forever, and routes the tile at every site.
    let use_q4 = q4 && use_tc && !hl;
    let tc_tj = if use_tc {
        // ★ The sequence tile is sized to the WIDEST PASS, not to the batch. A speculative verify
        // carries 2x batch rows (draft + current), so sizing from `batch` alone made the verify pay TWO
        // full weight passes for two tokens and MTP measured a dead-even 933.51 against the baseline's
        // 936.89. Sizing it to the verify turns that into one pass: 1001.35, +6.9%, ACCEPT 0.906.
        // The tile was never wrong about the batch — it was wrong about which pass is widest.
        let tj_rows = if mtp && prefill_t >= 2 {
            2 * batch
        } else {
            batch
        };
        let auto = tj_rows.next_power_of_two().clamp(8, 64);
        match std::env::var("LOGOS_Q38B_TC_TJ")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
        {
            Some(v) if v.is_power_of_two() && (8..=64).contains(&v) && v >= tj_rows => v,
            _ => auto,
        }
    } else {
        32
    };
    // ── HOW MUCH K TO STAGE PER BARRIER ROUND, CHOSEN BY SHAPE ──────────────────────────────────
    // Measured on an H100 NVL (2026-09-20), single-pass lane, 512 in / 128 out, aggregate tok/s:
    //   B=8   KC=128 313.36   KC=64 283.19
    //   B=32  KC=64  693.97   KC=128 677.66   KC=32 583.78   (repeat of KC=64: 693.23, so ~0.1% noise)
    //   B=64  KC=64  854.64   KC=128 826.47
    // The optimum MOVES with the batch: a wider sequence tile stages more activation per round, so the
    // round that best hides its own latency gets shorter. One constant would have been wrong at two of
    // the three shapes, which is the same argument that makes the GEMV/GEMM choice shape-dependent.
    // TC_KC must stay a multiple of 32 — see the static_assert in the tile; 48 measured FASTEST and was
    // computing a different model.
    let tc_kc_env: Option<usize> = std::env::var("LOGOS_Q38B_TC_KC")
        .ok()
        .and_then(|s| s.parse().ok());
    // ── HOW MANY BLOCKS PER SM ────────────────────────────────────────────────────────────────────
    // `__launch_bounds__(256, TC_OCC)`. The campaign recorded occupancy forcing as monotonically worse
    // (669.35 -> 668.38 -> 625.64 -> 555.41 -> 442.22) and stopped there; read again, that series says
    // the SECOND rung is free (-0.1%) and only the third onward collapses. It does collapse -- asking
    // for three blocks drives ptxas from 130 registers to 80 and the spills cost more than the
    // occupancy returns (156 vs 199 GB/s). But the second rung costs THREE registers (130 -> 127),
    // doubles residency from 17% to 33%, and is worth +19.2% of weight bandwidth at the decode shape
    // (199 vs 167 GB/s, three repeats, spread <1%). Register-bound at one block per SM is the state
    // this tile was actually in, and no amount of staging depth or split-K addressed it: split-K
    // plateaus dead flat from 80 to 640 blocks precisely because the blocks were never the shortage.
    // Measured on BOTH chips, because they disagree and the difference is 50%. At the decode shape
    // (K=17408, N=5120, B=32) with the split the rule picks:
    //
    //   MINBLOCKS      1      2      3      4      5
    //   RTX 3060     167    199    156      -      -     GB/s of weights   (48 KB SM, 1536 threads)
    //   H100 NVL     766   1045   1148   1026    912     GB/s of weights   (228 KB SM, 2048 threads)
    //
    // Ampere's third rung is a spill cliff -- ptxas drops from 130 registers to 80 and the reloads
    // cost more than the residency returns. Hopper has the register file to hold three blocks without
    // spilling, and it needs them: it has 4.7x the bandwidth to saturate and twice the threads per SM
    // to do it with. A single constant would give up 15% on one chip or 10% on the other, so the
    // choice is made by the ARCH MACRO and the emitted program tunes itself to whatever it is built
    // for. `__launch_bounds__` is the only consumer, and it is device-only, so the guard is honest.
    let tc_occ_def = match std::env::var("LOGOS_Q38B_TC_OCC")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
    {
        Some(n) => format!("#define TC_MINBLOCKS {n}"),
        None => [
            "#if defined(__CUDA_ARCH__) && __CUDA_ARCH__ >= 900",
            "#define TC_MINBLOCKS 3",
            "#else",
            "#define TC_MINBLOCKS 2",
            "#endif",
        ]
        .join("\n"),
    };
    // ── CHUNKED PREFILL ─────────────────────────────────────────────────────────────────────────
    // How many prompt tokens one prefill pass consumes. 1 is the shipped behaviour: the T=1 decode
    // graph replayed once per prompt token, which reads the 24.33 GB weight stream ONCE PER TOKEN.
    // At 512 prompt tokens that is TTFT 22.96 s against vLLM's 1.89 s, and it is most of a 5.12x
    // end-to-end serving deficit that no GEMM tuning can touch.
    //
    // Within a layer only the RECURRENCE is sequential in T: the norm, the projections and the whole
    // FFN are per-row and go through one GEMM at M = B*T, so the weights are read once per CHUNK.
    // Rows are laid out TOKEN-MAJOR (t*B + b) so token t's rows are contiguous and the stateful
    // kernels — which all index `base + bb*stride` — need only a base-pointer offset, keeping their
    // batch at B and their bodies untouched.
    // PT=1 emits exactly the code that produced every number on record.
    let prefill_t: usize = prefill_t.max(1);
    // Speculative decode needs a T>1 forward to verify a draft in ONE pass over the weights, which is
    // exactly what the chunked-prefill lane already builds. Asking for MTP therefore implies it.
    let mtp: bool = mtp && prefill_t >= 2;
    // The output tile is TI x TC_TJ with TI = 1024*TC_WM/TC_TJ, so a NARROW sequence tile makes a TALL
    // row tile out of the same 8 warps — and at TC_TJ=8 the measured TC_WM=4 would need 66 KB of
    // shared memory against the 48 KB static budget. Pick the widest sub-tile that actually fits
    // rather than pinning one and excluding the small batches from the lowering.
    // ── SUB-TILE HEIGHT, CHOSEN BY SHAPE ────────────────────────────────────────────────────────
    // The search below takes the tallest sub-tile that fits 48 KB, and it used to start at 4 — which
    // meant 8 was never considered even though TC_KC=64 leaves most of the budget unused. Measured on
    // an H100 NVL (2026-09-20), single-pass lane, 512 in / 128 out, token stream verified identical:
    //   B=32  WM=4 693.59   WM=8 657.35     <- the taller tile LOSES
    //   B=64  WM=4 862.90   WM=8 881.69     <- and WINS, by 2.2%
    // A taller tile does more mma per staged weight but halves the block count, so it pays only once
    // the batch is wide enough to keep the machine full without those blocks. Like TC_KC, one constant
    // would be wrong at one of the two shapes.
    // The taller sub-tile at batch 64 was measured when TC_KC was pinned to 64 for BOTH lanes, and it
    // won there by 2.2% (881.69 vs 862.90). It no longer does: TI=128 leaves no room for two stages of
    // the wide chunk, so WM=8 forces KC back to 64 and gives up more than its height returns. Measured
    // on an H100 NVL, single-pass, 512 in / 128 out, token stream verified identical both ways:
    //   B=64  WM=8 + KC=64  1067.98 tok/s      WM=4 + KC=128  1159.36 tok/s   <- +8.6%
    // A knob measured under a constraint that has since moved is not a measurement any more.
    let wm_start: usize = 4;
    // Sub-tile height and chunk width are chosen TOGETHER, because each one's budget depends on the
    // other and picking them in sequence made the first choice ration shared memory the second then
    // could not use. `act` is 2 only for the two-pass body, which stages Xh and Xl; the single-pass
    // lane reserves half as much and that is what lets it take the wide chunk.
    //
    // The +1 that used to sit in this budget was padding the XOR swizzle replaced. It over-reserved
    // by TI*4 + TJ*act*4 bytes a stage and was quietly excluding configurations that do fit.
    let act = if hl { 2 } else { 1 };
    let fits = |wm: usize, kc: usize, st: usize| -> bool {
        let (ti, sw) = (16 * (8 / (tc_tj / 8)) * wm, kc / 4);
        // ★ The Q4 lane stages a PACKED tile (`sP`, TI x KC/8 ints) beside the expanded one, and that
        // term was missing here: the height was chosen against a budget Q4 does not live in, so the
        // emitter picked a tile that fit WITHOUT Q4 and ptxas then rejected the program —
        // "uses too much shared data (0xec00 bytes, 0xc000 max)". Caught the moment the Hopper compile
        // gate was extended to the Q4 corner, which is the whole reason a gate covers every lane it
        // can emit rather than every lane someone remembered to list.
        let q4_extra = if use_q4 { st * ti * (kc / 8) * 4 } else { 0 };
        st * (ti * sw * 4 + act * tc_tj * sw * 4 + tc_tj * (kc / 32) * 4 + ti * (kc / 32) * 2)
            + q4_extra
            <= 48 * 1024
    };
    // Height is chosen EXACTLY as before -- the one-stage budget at the baseline chunk width, with the
    // stale `+1` padding kept deliberately. That padding was superseded by the XOR swizzle and now
    // over-reserves, but it is what every recorded WM measurement was taken under (B=32 WM=4 693.59 vs
    // WM=8 657.35; B=64 WM=8 881.69 vs WM=4 862.90), and widening a budget silently re-tunes a knob
    // that was measured end to end. Left as-is on purpose.
    let kc_base: usize = if batch >= 32 { 64 } else { 128 };
    let tc_wm: usize = std::env::var("LOGOS_Q38B_TC_WM")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| {
            let sw = kc_base / 4 + 1;
            *[wm_start, 4, 2, 1]
                .iter()
                .find(|wm| {
                    let ti = 16 * (8 / (tc_tj / 8)) * *wm;
                    ti * sw * 4
                        + 2 * tc_tj * sw * 4
                        + tc_tj * (kc_base / 32) * 4
                        + ti * (kc_base / 32) * 2
                        <= 48 * 1024
                })
                .expect("some sub-tile height fits the shared-memory budget")
        });
    let tc_ti = 16 * (8 / (tc_tj / 8)) * tc_wm;
    // TC_KC: the baseline, UPGRADED to the wide chunk only where the running lane's budget has room
    // for two stages of it. This can only widen a program that already fit, never re-tune one.
    // Measured on an H100 NVL at the decode shape with bq_sk_reduce INCLUDED: KC=128 reads 1320 GB/s
    // against KC=64's 1077 -- 23%, and unreachable until the unused two-pass wrapper stopped being
    // compiled beside it and rationing the 48 KB against a lane that never runs.
    let tc_kc: usize = tc_kc_env.unwrap_or_else(|| {
        if kc_base < 128 && fits(tc_wm, 128, 2) {
            128
        } else {
            kc_base
        }
    });
    // ── STAGED K-CHUNKS, CHOSEN BY WHAT FITS ────────────────────────────────────────────────────
    // 2 means the weights and activations for the next chunk are in flight via 16-byte `cp.async`
    // while this chunk's mma runs, instead of being held in registers. MEASURED on an A100-SXM4,
    // swizzled layout, golden at every point:
    //   B=32  515.86 -> 565.28  (+9.6%)     B=64  553.82 -> 650.24  (+17.4%)
    // It is not free: every staged array doubles. At B=8 the tile is TALL (TI=256) and two stages
    // need 64 KB against the 48 KB static budget -- the tile's own static_assert rejects it, which is
    // the right outcome but a compile error is a poor default. So the budget is evaluated HERE, with
    // the same arithmetic the assert uses, and staging is requested only where it actually fits.
    // ★ The budget must be the WORST instantiation, not this lane's. `bq_gemm_tc` and `bq_gemm_tc1`
    // are BOTH emitted into every program, so the body is instantiated at TWOPASS=true and false, and
    // ptxas compiles both regardless of which one is launched. The two-pass instantiation carries the
    // extra `sL` activation buffer. Using this lane's `hl` here let the batch-1 fast lane ask for two
    // stages on a budget only the single-pass instantiation fits: 44,032 B computed against 52,224 B
    // actually required, and the tile's own static_assert rejected the program at nvcc time.
    // So: always price the two-pass case. (Parenthesised deliberately — `if c {2} else {1} * x` parses
    // as `if c {2} else {1*x}` in Rust, which is its own quiet defect.)
    let stage_bytes_kc = |kc: usize, st: usize| -> usize {
        let sw = kc / 4;
        st * (tc_ti * sw * 4 + act * tc_tj * sw * 4 + tc_tj * (kc / 32) * 4 + tc_ti * (kc / 32) * 2)
    };
    // ★ Q4 CANNOT RIDE THE ONE-STAGE PATH. At TC_STAGES==1 the tile stages weights through registers
    // (`BQTC_GLOAD`), which reads at the int8 stride and fills `sW` -- while the Q4 fragment reader
    // takes its nibbles from `sP`, which nothing then writes. That combination reads ~44 MB past a
    // packed buffer and, where it survives, multiplies an uninitialised tile. A `static_assert` in the
    // kernel now refuses it outright, so the emitter must not ask for it.
    //
    // Q4 also pays for BOTH shared tiles: `sW` is declared unconditionally and `sP` sits beside it, so
    // a stage costs the int8 tile plus a half-width packed one. At TI=128 that is 56 KB at KC=128 and
    // 29 KB at KC=64 -- which is why the Hopper gate once caught `0xec00 vs 0xc000`. Shrink the chunk
    // until two Q4 stages fit rather than silently dropping to a depth Q4 cannot use.
    let q4_stage_bytes =
        |kc: usize, st: usize| -> usize { stage_bytes_kc(kc, st) + st * tc_ti * (kc / 8) * 4 };
    let tc_kc = if use_q4 {
        let mut kc = tc_kc;
        while kc > 32 && q4_stage_bytes(kc, 2) > 48 * 1024 {
            kc /= 2;
        }
        kc
    } else {
        tc_kc
    };
    let stage_bytes = |st: usize| -> usize { stage_bytes_kc(tc_kc, st) };
    let tc_stages: usize = std::env::var("LOGOS_Q38B_TC_STAGES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(if use_q4 {
            2
        } else if stage_bytes(2) <= 48 * 1024 {
            2
        } else {
            1
        });
    // Two stages, and the reason is occupancy rather than depth. Measured on a 3060 at the decode
    // shape (K=17408, N=5120, B=32), depth 3 reads +10.4% over depth 2 -- but the cause is not the
    // deeper pipeline. At 256 threads the tile needs <=128 registers to fit TWO blocks on an SM, and
    // depth 2 compiles to 130: it misses by TWO REGISTERS and runs at one block, 17% occupancy. Depth
    // 3 happens to compile to 123 and crosses the line. Asking for the second block directly (TC_OCC,
    // below) is worth more -- 199 vs 184 GB/s -- and then depth 3 LOSES, because its 33 KB of shared
    // against depth 2's 22 KB is spent on the same budget the second block comes out of.
    // ── HOW FAR TO SPLIT K ────────────────────────────────────────────────────────────────────────
    // Not every projection is wide. N=5120 is 128 of the 257 GEMM launches in a step and yields only
    // 40 blocks of the 128-row tile, so 70% of an H100's SMs idle and the tile's speed is irrelevant.
    // Aim at ~1024 blocks — comfortably several waves of 132 SMs — but never slice K so thin that a
    // slice is under two staged chunks, and never past 16 because the partial-sum traffic starts to
    // cost more than the idle SMs did. Measured against a per-shape hand-tuned split this rule is
    // within 1% (33.6 ms vs 33.2 ms of GEMM per step), so a table would buy nothing it does not.
    // How many blocks the split rule aims to put on the machine. Splitting K buys occupancy and costs
    // TRAFFIC: each split writes SPLIT x N x B floats of partials to global and `bq_sk_reduce` reads
    // them back. COUNTED EXACTLY off the emitted B=32 lane (257 tile launches, 256 of them split), not
    // estimated: **7.16 GB of partials against 25.60 GB of weights — 21.9% of the GEMM's memory traffic
    // exists only because the reduction is split.** The worst shape is K=6144,N=5120,SPLIT=16: 21.0 MB
    // of partials against 31.5 MB of weights, a 67% surcharge, x64 launches a step.
    //
    // The rule prices that at ZERO — it picks SPLIT to maximise BLOCK COUNT. And 1024 was tuned on the
    // TWO-pass lane, which carries twice the activation traffic; the single-pass lane we now ship has a
    // different balance, so the inherited constant is tuned for a kernel we do not run.
    //
    // Modelled off the real shapes, TARGET=768 gives the SAME worst-case block count (640, set by the
    // N=5120 shapes hitting the clamp at 16) for 1.15 GB less traffic, and 384 gives 400 blocks for
    // 3.50 GB less. SWEEP IT — the trade is a measurement, not a derivation.
    let tc_target: usize = std::env::var("LOGOS_Q38B_TC_TARGET")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1024);
    // The clamp, not TARGET, is what binds at the narrow shapes: N=5120 gives base=40, so
    // ceil(1024/40)=26 is cut to 16 and those 128 launches a step get 640 blocks. The H100 sweep
    // measured the trade running the OPPOSITE way from the traffic model -- 640 blocks beat 512 beat
    // 384 beat 256 even though each step DOWN moved fewer bytes -- which says the tile is short of
    // blocks to hide latency, not short of bandwidth. If that reading is right, the lever is to raise
    // the ceiling, not lower the target. Default 16 (shipped, measured); a knob so the other direction
    // can be measured rather than argued.
    let tc_split_max: usize = std::env::var("LOGOS_Q38B_TC_SPLIT_MAX")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(16);
    let tc_split = move |k: usize, n: usize| -> usize {
        if !use_tc {
            return 1;
        }
        let base = n.div_ceil(tc_ti);
        tc_target
            .div_ceil(base)
            .clamp(1, tc_split_max)
            .min(((k / tc_kc) / 2).max(1))
    };
    // The partial buffer is sized from the sites that actually split, recorded as they are emitted.
    let tc_part_max = std::cell::Cell::new(0usize);

    let mmq_min: usize = std::env::var("LOGOS_Q38B_MMQ_MIN")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(32);
    let use_mmq = !use_mma && !use_staged && !use_tc && batch >= mmq_min;
    // The sequence tile must be a multiple of MMQ_RJ (=2) and cover the batch.
    // The fallback must be a tile that COMPILES, not a placeholder. `bq_gemm_mmq` is emitted into every
    // lane whether or not it is launched, and ptxas compiles every `__global__` in the translation unit,
    // so its defines have to be legal even when the shape rule picked a different lowering. It derives
    // `TI = (256/(MMQ_TJ/MMQ_RJ))*MMQ_RI`, so the old fallback of 2 gave TI=512 and a 133 KB `sW` against
    // the 48 KB static budget — `ptxas error: uses too much shared data (0x22c50, 0xc000 max)`. 32 is the
    // kernel's own `#ifndef` default, so the emitted value and the built-in one cannot disagree.
    let mmq_tj = if use_mmq { batch.div_ceil(2) * 2 } else { 32 };
    let mmq_acc = std::env::var("LOGOS_Q38B_MMQ_ACC").unwrap_or_else(|_| "double".to_string());
    let gemv_rows = |y: &str,
                     w: &str,
                     xh: &str,
                     xs: &str,
                     k: usize,
                     n: usize,
                     ys: usize,
                     rows: &str|
     -> String {
        // `rows` is "B" for decode and "BT" for a chunked pass. Only the tensor-core branch honoured
        // it; the rest hardcoded B and sized their grid from `batch`, so a chunk lowered through them
        // computed its first B rows and left the rest as whatever was in the buffer. Invisible while
        // the prefill lane always took the rows-in-M tile, and a silently wrong second row the moment
        // a narrow chunk routed elsewhere -- which is exactly what a speculative verify is.
        let nrows = if rows == "BT" {
            batch * prefill_t
        } else {
            batch
        };
        let btiles = nrows.div_ceil(BTG);
        if use_mma {
            // GEMM lowering: grid is (N tiles, M tiles), 256 threads = 8 warps of m16n8k32.
            return format!("bq_gemm_mma<<<dim3(({n}+TBN-1)/TBN,({nrows}+TBM-1)/TBM),256,(size_t)(TBM+TBN)*(KSTEP+4),s>>>({y},{w}_q,{w}_s,{xh},{xs},{k},{n},{rows},{ys});");
        }
        if use_wide {
            // Row-parallel: MV_RB output rows per block, MV_NW warps reducing K together. N/MV_RB
            // blocks already fills the machine at every real shape, so K is not sliced here.
            return format!("bq_gemv_wide<<<dim3(({n}+MV_RB-1)/MV_RB,1),MV_NW*32,0,s>>>({y},{w}_q,{w}_s,{xh},{xh}l,{xs},{k},{n},{rows},{ys},(float*)0,1);");
        }
        if use_tc {
            let sp = tc_split(k, n);
            let blocks = n.div_ceil(tc_ti);
            // One tile, two instantiations: `bq_gemm_tc` consumes hi+lo, `bq_gemm_tc1` hi alone.
            // One tile, three instantiations: hi+lo, hi alone, and hi against Q4 weights. The Q4 path
            // takes the SAME argument shape, so only the kernel name and the two weight buffers move.
            // ...and the site must agree with the loader about which buffers exist.
            let use_q4 = use_q4 && w != "emb" && w != "Wout";
            let (tck, xops) = if use_q4 {
                ("bq_gemm_tc1_q4", format!("{xh}"))
            } else if hl {
                ("bq_gemm_tc", format!("{xh},{xh}l"))
            } else {
                ("bq_gemm_tc1", format!("{xh}"))
            };
            let (wq, ws) = if use_q4 {
                (format!("{w}_q4"), format!("{w}_s4"))
            } else {
                (format!("{w}_q"), format!("{w}_s"))
            };
            // grid.z walks the rows in TC_TJ-wide M-tiles, so `rows` may exceed the sequence tile.
            let mt = format!("(({rows})+TC_TJ-1)/TC_TJ");
            if sp > 1 {
                tc_part_max.set(tc_part_max.get().max(sp * n));
                return format!(
                    "{tck}<<<dim3({blocks},{sp},{mt}),256,0,s>>>({y},{wq},{ws},{xops},{xs},{k},{n},{rows},{ys},skp,{sp});\n\
                     \x20 bq_sk_reduce<<<Gs({n}*({rows})),256,0,s>>>({y},skp,{n},{rows},{ys},{sp});");
            }
            return format!("{tck}<<<dim3({blocks},1,{mt}),256,0,s>>>({y},{wq},{ws},{xops},{xs},{k},{n},{rows},{ys},(float*)0,1);");
        }
        if use_mmq {
            // One block per TI output rows; TI is derived inside the kernel from the thread budget.
            return format!("bq_gemm_mmq<<<(({n})+((256/(MMQ_TJ/MMQ_RJ))*MMQ_RI)-1)/((256/(MMQ_TJ/MMQ_RJ))*MMQ_RI),256,0,s>>>({y},{w}_q,{w}_s,{xh},{xh}l,{xs},{k},{n},{rows},{ys});");
        }
        if use_staged {
            // Staged dp4a: one warp per output row (full parallelism kept), activations from shared memory.
            if hl {
                return format!("bq_gemv_hl_smem<<<({n}+7)/8,256,(size_t)(2*{nrows}*512+{nrows}*16*4),s>>>({y},{w}_q,{w}_s,{xh},{xh}l,{xs},{k},{n},{rows},{ys});");
            }
            return format!("bq_gemv_h_smem<<<({n}+7)/8,256,(size_t)({nrows}*512+{nrows}*16*4),s>>>({y},{w}_q,{w}_s,{xh},{xs},{k},{n},{rows},{ys});");
        }
        if hl {
            format!("bq_gemv_hl<<<dim3(Gw({n}),{btiles}),256,0,s>>>({y},{w}_q,{w}_s,{xh},{xh}l,{xs},{k},{n},{rows},{ys});")
        } else {
            format!("bq_gemv_h<<<dim3(Gw({n}),{btiles}),256,0,s>>>({y},{w}_q,{w}_s,{xh},{xs},{k},{n},{rows},{ys});")
        }
    };

    // Decode's view: one row per sequence. Prefill will pass "BT".
    let gemv = |y: &str, w: &str, xh: &str, xs: &str, k: usize, n: usize, ys: usize| -> String {
        gemv_rows(y, w, xh, xs, k, n, ys, "B")
    };

    // Consumer-side split-K fusion. A split GEMM writes S partial planes and a `bq_sk_reduce` launch
    // sums them into Y; but three of the four consumers in a layer read that Y exactly once, so they
    // can sum the partials themselves. That deletes the launch AND a full write+read of Y. Folding the
    // reduction into the PRODUCER was measured 9% slower (it funnels each tile into one block); the
    // consumer already owns a full grid, so nothing serialises.
    let drop_reduce = |launch: String| -> String {
        match launch.rfind("bq_sk_reduce") {
            Some(i) => launch[..i].trim_end().to_string(),
            None => launch,
        }
    };
    // Which consumers may fold the split-K sum in. MEASURED, not assumed: folding into the RMSNorm is
    // 3.7% SLOWER, because that kernel runs ONE BLOCK PER SEQUENCE -- 32 blocks against the 640 the
    // standalone reduce spreads over -- so it starves the very reduction it absorbed. The SiLU epilogue
    // runs 2176 blocks at the same shape and has parallelism to spare. Concentration, not launch count,
    // is what decides a fusion here; `all` is kept only so the negative stays reproducible.
    // `0` = none, `silu` = the wide consumer only (default), `all` = every consumer.
    let skf = std::env::var("LOGOS_Q38B_SKFUSE").unwrap_or_else(|_| "silu".to_string());
    let sk_fuse = skf != "0";
    // Same shape at every layer, so the split is a constant per site.
    let sk_norm = !norm_split && (skf == "all" || skf == "1");
    let wd_sp = if sk_norm { tc_split(dff, d) } else { 1 };
    let wo_attn_sp = if sk_norm { tc_split(aq, d) } else { 1 };
    let wo_gdn_sp = if sk_norm { tc_split(vw, d) } else { 1 };
    let gu_sp = if sk_fuse { tc_split(d, 2 * dff) } else { 1 };

    let mut decls = String::new();
    let mut loads = String::new();
    // The DeepGEMM lane's per-matrix state. Emitted always, compiled only under BQ_ENABLE_DG, so the
    // default program is byte-for-byte what it was and the lane is a BUILD choice rather than an
    // emitter flag — one .cu, two lanes.
    let mut dg_decls = String::new();
    let mut dg_loads = String::new();
    let mut q8 = |var: &str, rows: usize, inn: usize, file: &str| {
        decls.push_str(&format!(
            "static signed char* {var}_q; static __half* {var}_s;\n"
        ));
        loads.push_str(&format!(
            "  LDQ({var},(size_t){rows}*{inn},(size_t){rows}*({inn}/32),\"{wdir}/{file}.q\",\"{wdir}/{file}.s\");\n"));
        // The embedding and the output head stay Q8_0, for two reasons that happen to agree: the
        // logits decide the token, so the head is the most precision-sensitive matmul in the model;
        // and `bq_embed` / the head launch read `_q`/`_s` directly rather than through `gemv_rows`, so
        // converting them would free buffers those sites still use. Together they are 2.54 GB of the
        // 24.33 GB weight set — ~10% of the bytes for all of the precision risk.
        let q4_this = use_q4 && var != "emb" && var != "Wout";
        if q4_this {
            decls.push_str(&format!(
                "static signed char* {var}_q4; static __half* {var}_s4;\n"
            ));
            loads.push_str(&format!(
                "  cudaMalloc(&{var}_q4,(size_t){rows}*{inn}/2); cudaMalloc(&{var}_s4,(size_t){rows}*({inn}/32)*2);\n\
                 \x20 bq_q8_to_q4<<<1024,256>>>((unsigned char*){var}_q4,{var}_s4,{var}_q,{var}_s,{rows},{inn});\n\
                 \x20 cudaDeviceSynchronize(); cudaFree({var}_q); cudaFree({var}_s); {var}_q=0; {var}_s=0;\n"));
        }
    };
    q8("emb", cfg.vocab, d, "emb");
    q8("Wout", cfg.vocab, d, "Wout");
    for l in 0..cfg.layers {
        if cfg.is_attn(l) {
            q8(&format!("qgkv{l}"), attnw, d, &format!("qgkv{l}"));
            q8(&format!("Wo{l}"), d, aq, &format!("Wo{l}"));
        } else {
            q8(&format!("qkvz{l}"), projw, d, &format!("qkvz{l}"));
            q8(&format!("Wo{l}"), d, vw, &format!("Wo{l}"));
        }
        q8(&format!("gu{l}"), 2 * dff, d, &format!("gu{l}"));
        q8(&format!("Wd{l}"), d, dff, &format!("Wd{l}"));
    }
    // ── MULTI-TOKEN PREDICTION ────────────────────────────────────────────────────────────────────
    // The checkpoint ships its own draft head: blk.{layers} is an ordinary gated-attention layer plus
    // FFN, fed by a 2E -> E adapter over [norm(embedding of the token just produced) ++ norm(residual
    // stream)]. Measured 2026-09-05 at 83.3% accept over 12 samples of ONE repetitive prompt, which is
    // an optimistic upper bound -- llama.cpp's +33-39% implies nearer 45-50% in the wild, and even 50%
    // is 1 + p tokens for one pass over the weights.
    //
    // Speculation is the right lever for THIS engine specifically. The ablation puts 64% of the step
    // in one GEMM whose efficiency tracks chunks-per-block, and ~5 ms in per-node graph overhead over
    // 1045 launches; neither gets cheaper by tuning, but both are paid PER STEP, and speculation makes
    // fewer steps carry the same tokens. vLLM's own MTP buys it +9.5% at 32 concurrent and is NEGATIVE
    // at 1 (72.49 vs 73.24) because it is already near its limits. We are not.
    if mtp {
        let l = cfg.layers;
        q8(&format!("qgkv{l}"), attnw, d, &format!("qgkv{l}"));
        q8(&format!("Wo{l}"), d, aq, &format!("Wo{l}"));
        q8(&format!("gu{l}"), 2 * dff, d, &format!("gu{l}"));
        q8(&format!("Wd{l}"), d, dff, &format!("Wd{l}"));
        q8("ehproj", d, 2 * d, "ehproj");
    }
    if mtp {
        decls.push_str(
            "static float *membed,*mcat,*mx,*mtmp,*m2n,*mf32,*dlogits,*mkc,*mvc,*mas,*vlogits;\n\
static signed char *maq,*maql;\nstatic int *d_draft,*d_verify,*d_acc;\n",
        );
    }
    let mut f32w = |var: &str, n: usize, file: &str| {
        decls.push_str(&format!("static float* {var}_f;\n"));
        loads.push_str(&format!("  LDF({var},(size_t){n},\"{wdir}/{file}.f\");\n"));
    };
    f32w("on", d, "on");
    if mtp {
        let l = cfg.layers;
        f32w(&format!("an{l}"), d, &format!("an{l}"));
        f32w(&format!("fn{l}"), d, &format!("fn{l}"));
        f32w(&format!("Qn{l}"), cfg.hd, &format!("Qn{l}"));
        f32w(&format!("Kn{l}"), cfg.hd, &format!("Kn{l}"));
        f32w("enorm", d, "enorm");
        f32w("hnorm", d, "hnorm");
        f32w("shnorm", d, "shnorm");
    }
    for l in 0..cfg.layers {
        f32w(&format!("an{l}"), d, &format!("an{l}"));
        f32w(&format!("fn{l}"), d, &format!("fn{l}"));
        if cfg.is_attn(l) {
            f32w(&format!("Qn{l}"), cfg.hd, &format!("Qn{l}"));
            f32w(&format!("Kn{l}"), cfg.hd, &format!("Kn{l}"));
        } else {
            f32w(&format!("Wc{l}"), taps * convc, &format!("Wc{l}"));
            f32w(&format!("ba{l}"), 2 * nv * d, &format!("ba{l}"));
            f32w(&format!("Dt{l}"), nv, &format!("Dt{l}"));
            f32w(&format!("Al{l}"), nv, &format!("Al{l}"));
            f32w(&format!("Gn{l}"), ghd, &format!("Gn{l}"));
        }
    }

    let mut mtp_roll = String::new();
    let mut state_decl = String::new();
    let mut state_alloc = String::new();
    for l in 0..cfg.layers {
        if cfg.is_attn(l) {
            state_decl.push_str(&format!("static bq_kv_t *kc{l}, *vc{l};\n"));
            state_alloc.push_str(&format!(
                "  cudaMalloc(&kc{l},(size_t)B*MAXSEQ*{akv}*sizeof(bq_kv_t)); cudaMalloc(&vc{l},(size_t)B*MAXSEQ*{akv}*sizeof(bq_kv_t));\n"));
        } else {
            if l == 0 {
                // ★ THE ABLATION MASK. Runtime, because the alternative is six nvcc runs on rented
                // time to answer one question — the same trade the DeepGEMM site bisect already paid.
                state_decl
                    .push_str("static int bq_da[5] = {0,0,0,0,0};  // gdn conv attn norm ffn\n");
            }
            state_decl.push_str(&format!("static bq_state_t *st{l}; static float *cs{l};\n"));
            state_alloc.push_str(&format!(
                "  cudaMalloc(&st{l},(size_t)B*{vw}*{ghd}*sizeof(bq_state_t)); cudaMemset(st{l},0,(size_t)B*{vw}*{ghd}*sizeof(bq_state_t));\n\
                 \x20 cudaMalloc(&cs{l},(size_t)B*{taps}*{convc}*4); cudaMemset(cs{l},0,(size_t)B*{taps}*{convc}*4);\n"));
            // The speculative rollback shadow. It doubles the recurrent state (4.8 GB at 32
            // sequences), which is the price of not paying a 4.8 GB snapshot BEFORE every verify for
            // a rollback that mostly will not happen. The conv ring gets one too: it is shifted once
            // per token, so a rejected token leaves it one shift ahead.
            if mtp {
                mtp_roll.push_str(&format!(
                    "  bq_mtp_rollback<<<dim3(64,B),256,0,s>>>(st{l},stmid{l},d_acc,(long){vw}*{ghd});\n\
                     \x20 bq_mtp_rollback<<<dim3(8,B),256,0,s>>>(cs{l},csmid{l},d_acc,(long){taps}*{convc});\n"));
                state_decl.push_str(&format!(
                    "static bq_state_t *stmid{l}; static float *csmid{l};\n"
                ));
                state_alloc.push_str(&format!(
                    "  cudaMalloc(&stmid{l},(size_t)B*{vw}*{ghd}*sizeof(bq_state_t)); cudaMemset(stmid{l},0,(size_t)B*{vw}*{ghd}*sizeof(bq_state_t));\n\
                     \x20 cudaMalloc(&csmid{l},(size_t)B*{taps}*{convc}*4); cudaMemset(csmid{l},0,(size_t)B*{taps}*{convc}*4);\n"));
            }
        }
    }

    // Only the gate/up projections are routed for now: they are the widest matmul in the model
    // (N=2*dff=34816), DeepGEMM measured 4.32x our tile at that shape, and their activation has an
    // f32 source (`x2n`, written by the FFN norm) so the site converts without a new intermediate.
    for l in 0..cfg.layers {
        for (wv, n, k) in [
            (format!("gu{l}"), 2 * dff, d),
            // the entry projection: its activation is `x_f32`, which the entry norm already writes
            (
                if cfg.is_attn(l) {
                    format!("qgkv{l}")
                } else {
                    format!("qkvz{l}")
                },
                if cfg.is_attn(l) { attnw } else { projw },
                d,
            ),
            // the FFN down projection: its activation is the SiLU output, which the fused producer
            // now writes as fp8 directly -- no f32 hidden buffer is ever materialised for it
            (format!("Wd{l}"), d, dff),
        ] {
            let (dc, ld) = dg_weight(&wv, n, k);
            dg_decls.push_str(&dc);
            dg_loads.push_str(&ld);
        }
    }
    // ★ THE SITE MASK IS A RUNTIME KNOB, NOT A BUILD FLAG. A routed lane that is faster and computes
    // a different function has to be BISECTED, and a build flag would make that four ten-minute nvcc
    // runs on a rented H100. One binary with a runtime mask isolates any site for the price of a rerun.
    dg_decls.push_str("static unsigned char* dgA; static float* dgAsf; static float* dgD;\n\
                       static int bq_dg_hl = 1;   // two-pass fp8 weights: accuracy parity with int8\n\
                       static int bq_dg_on[3] = {1,1,1};   // 0 = gate/up, 1 = entry projection, 2 = down\n");
    // Without DeepGEMM the mask still exists — constant OFF — so every routed site can be written as ONE
    // `if(!bq_dg_on[i]) { int8 } else { fp8 }` with only the fp8 body conditional. The earlier form
    // emitted the int8 fallback twice (inside the runtime branch and again under `#else`), which put two
    // matmul lowerings on every routed projection and turned the emit census red — unnoticed, because
    // the drivers treated "wrote" in the output as success and the file is written before it asserts.
    dg_loads.push_str(&format!(
        "  cudaMalloc(&dgA,(size_t)BT*{w});\n\
         \x20 cudaMalloc(&dgAsf,(size_t)({w}/BQ_DG_BLOCK_K)*bq_dg_align(BT,4)*4);\n\
         \x20 cudaMalloc(&dgD,(size_t)BT*{wn}*4);\n",
        w = d.max(dff),
        wn = (2 * dff).max(projw).max(attnw)
    ));

    // ── `LOGOS_Q38B_DABLATE` — ATTRIBUTION BY DELETION, FOR DECODE ──────────────
    // Prefill got this treatment and decode never did, which is backwards: decode is the metric we
    // lose, and its composition is the largest unknown left in this campaign. The GEMM accounts for
    // ~21 ms of a ~34 ms step (measured rate x known byte count), and the other ~12 ms has never been
    // attributed to anything. Every trustworthy decision here came from deleting a phase and reading
    // the difference on the target chip; every retraction came from reasoning about one instead — the
    // split-K traffic model died the same day it was written, once its partials turned out to fit L2.
    //
    // Groups mirror the decode kernel census (965 launches a step): `gdn` is the gates plus the
    // recurrence, `conv` the depthwise conv and gated norm, `attn` the whole attention block, `norm`
    // the residual-add RMSNorm, `ffn` the gate/up + SiLU + down chain. The output is wrong by
    // construction — this is a stopwatch, not a lane — and the equivalence locks never see it because
    // the default is "none". One boot, six runs, and the 12 ms stops being a bucket.
    // ★ RUNTIME, NOT A BUILD FLAG — the lesson this session already paid for once. Bisecting the
    // DeepGEMM sites as a compile-time switch would have been four ten-minute nvcc runs on a rented
    // H100; as a runtime mask it was one build and N runs. Six ablation groups is the same trade at
    // six times the size. The decode graph is captured ONCE at startup, so a host-side `if` around a
    // launch is resolved during capture and the captured graph simply lacks that node — the ablation
    // is as exact as deleting the emit, for the price of one build.
    let da = |g: usize| -> String { format!("if(!bq_da[{g}]) ") };
    let (da_gdn, da_conv, da_attn, da_norm, da_ffn) = (da(0), da(1), da(2), da(3), da(4));

    let mut step = String::new();
    for l in 0..cfg.layers {
        let wo_sp = if cfg.is_attn(l) {
            wo_attn_sp
        } else {
            wo_gdn_sp
        };
        let entry = if l == 0 {
            "bq_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,aqE,aqEl,asE,x_f32,an0_f,E,EPSV);".to_string()
        } else {
            if norm_split {
                format!("bq_add_rmsnorm_part<<<dim3(NORM_G,B),RMSN_NT,0,s>>>(x,tmp,normp,E,NORM_G);\n\
                         \x20 bq_add_rmsnorm_apply<<<dim3(NORM_G,B),RMSN_NT,0,s>>>(x,normp,aqE,aqEl,asE,x_f32,an{l}_f,E,EPSV,NORM_G);")
            } else {
                if wd_sp > 1 {
                    format!("bq_add_rmsnorm_q8f_hl_sk<<<B,RMSN_NT,0,s>>>(x,skp,{wd_sp},aqE,aqEl,asE,x_f32,an{l}_f,E,EPSV);")
                } else {
                    format!("bq_add_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,tmp,aqE,aqEl,asE,x_f32,an{l}_f,E,EPSV);")
                }
            }
        };
        step.push_str(&format!("  {entry}\n"));
        if cfg.is_attn(l) {
            step.push_str(&format!(
                "  {g_qgkv}\n\
                 {ablk}\
                 \x20 {g_wo}\n",
                ablk = format!(
                 "\x20 {da_attn}{{\n\
                  \x20 bq_kv_store<<<dim3(Gs({akv}*2),B),256,0,s>>>(kc{l},vc{l},d_pos,{akv},pbuf+{two_aq},PMAX);\n\
                  \x20 bq_qk_norm_rope_part<<<dim3({h},B),128,0,s>>>(pbuf,d_zero,0,PMAX,Qn{l}_f,{h},{hd},{rot},ROPEV,EPSV);\n\
                  \x20 bq_qk_norm_rope_kv<<<dim3({kv},B),128,0,s>>>(kc{l},d_pos,{akv},MAXSEQ*{akv},Kn{l}_f,{kv},{hd},{rot},ROPEV,EPSV);\n\
                  \x20 bq_attn_launch(s,attnb,pbuf,PMAX,kc{l},vc{l},d_pos,{h},{kv},{hd},SCALEV,B);\n\
                  \x20 bq_gate_mul_q8_hl<<<dim3(Gs({aq}),B),256,0,s>>>(attnb,{aq},pbuf+{aq},PMAX,aqQ,aqQl,asQ,{aq});\n\
                  \x20 }}\n",
                  da_attn = da_attn, aq = aq,
                  akv = akv, l = l, two_aq = 2 * aq, h = cfg.h, kv = cfg.kv, hd = cfg.hd, rot = cfg.rope_dim),
                g_qgkv = gemv("pbuf", &format!("qgkv{l}"), "aqE", "asE", d, attnw, projw.max(attnw)),
                g_wo = if wo_sp > 1 { drop_reduce(gemv("tmp", &format!("Wo{l}"), "aqQ", "asQ", aq, d, d)) }
                       else { gemv("tmp", &format!("Wo{l}"), "aqQ", "asQ", aq, d, d) }));
        } else {
            step.push_str(&format!(
                "  {g_qkvz}\n\
                 \x20 bq_gemv_f32w<<<dim3(({two_nv}+(8/F32W_NW)-1)/(8/F32W_NW),B),256,0,s>>>(gbuf,ba{l}_f,x_f32,E,{two_nv},B);\n\
                 {convk}\
                 \x20 {gdn}\n\
                 {gnorm}\
                 \x20 {g_wo}\n",
                convk = format!(
                    "\x20 {da_conv}{{\n\x20 bq_conv_silu_l2<<<dim3({convc}/{ghd},B),{ghd},0,s>>>(pbuf,PMAX,Wc{l}_f,cs{l},cbuf,{convc},{taps},{two_nk},EPSV);\n\x20 }}\n",
                    da_conv = da_conv, convc = convc, ghd = ghd, l = l, taps = taps, two_nk = 2 * nk),
                gnorm = format!(
                    "\x20 {da_conv}{{\n\x20 bq_gate_norm_hl<<<dim3({nv},B),{ghd},0,s>>>(pbuf+{qkv_off},Gn{l}_f,obuf,gnbuf,aqQ,aqQl,asQ,{ghd},{nv},PMAX,EPSV);\n\x20 }}\n",
                    da_conv = da_conv, nv = nv, ghd = ghd, l = l, qkv_off = 2 * qk + vw),
                gdn = { let two_qk = 2 * qk;
                    format!("{da_gdn}{{\n\x20 bq_gdn_gates<<<dim3(Gs({nv}),B),256,0,s>>>(gbuf,Dt{l}_f,Al{l}_f,betab,decayb,{nv});\n\
                             \x20 bq_gdn_chunk<<<dim3({nv},BQ_GDN_CHUNK_GY({ghd}),B),128,0,s>>>(cbuf,cbuf+{qk},cbuf+{two_qk},betab,decayb,st{l},obuf,{ghd},{nk},{nv},{convc},1,B);\n\x20 }}") },
                g_qkvz = gemv("pbuf", &format!("qkvz{l}"), "aqE", "asE", d, projw, projw.max(attnw)),
                g_wo = if wo_sp > 1 { drop_reduce(gemv("tmp", &format!("Wo{l}"), "aqQ", "asQ", vw, d, d)) }
                       else { gemv("tmp", &format!("Wo{l}"), "aqQ", "asQ", vw, d, d) },
                two_nv = 2 * nv));
        }
        step.push_str(&format!(
            "  {da_norm}{{\n\x20 {norm_ffn}\n\x20 }}\n\
             \x20 {da_ffn}{{\n\
             \x20 {g_gu}\n\
             \x20 {silu}\n\
             \x20 {g_wd}\n\
             \x20 }}\n\
             \x20 {resid}",
            da_norm = da_norm, da_ffn = da_ffn,
            norm_ffn = if norm_split {
                format!("bq_add_rmsnorm_part<<<dim3(NORM_G,B),RMSN_NT,0,s>>>(x,tmp,normp,E,NORM_G);\n\
                         \x20 bq_add_rmsnorm_apply<<<dim3(NORM_G,B),RMSN_NT,0,s>>>(x,normp,ffQ,ffQl,ffS,x2n,fn{l}_f,E,EPSV,NORM_G);")
            } else if wo_sp > 1 {
                format!("bq_add_rmsnorm_q8f_hl_sk<<<B,RMSN_NT,0,s>>>(x,skp,{wo_sp},ffQ,ffQl,ffS,x2n,fn{l}_f,E,EPSV);")
            } else {
                format!("bq_add_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,tmp,ffQ,ffQl,ffS,x2n,fn{l}_f,E,EPSV);")
            },
            silu = if gu_sp > 1 {
                format!("bq_silu_had_q8_hl_sk<<<dim3(Gs({dff}),B),256,0,s>>>(skp,{gu_sp},hqQ,hqQl,hqS,{dff});")
            } else {
                format!("bq_silu_had_q8_hl<<<dim3(Gs({dff}),B),256,0,s>>>(ffbuf,hqQ,hqQl,hqS,{dff});")
            },
            g_gu = if gu_sp > 1 { drop_reduce(gemv("ffbuf", &format!("gu{l}"), "ffQ", "ffS", d, 2 * dff, 2 * dff)) }
                   else { gemv("ffbuf", &format!("gu{l}"), "ffQ", "ffS", d, 2 * dff, 2 * dff) },
            g_wd = if wd_sp > 1 && l + 1 < cfg.layers { drop_reduce(gemv("tmp", &format!("Wd{l}"), "hqQ", "hqS", dff, d, d)) }
                   else { gemv("tmp", &format!("Wd{l}"), "hqQ", "hqS", dff, d, d) },
            resid = if l + 1 == cfg.layers { "bq_add<<<dim3(Gs(E),B),256,0,s>>>(x,tmp,E);\n" } else { "\n" }));
    }

    // The prefill chunk. Emitted only when PT>1; a separate function so `decode_step` is untouched.
    // ── PREFILL ABLATION ────────────────────────────────────────────────────────────────────────
    // `LOGOS_Q38B_PABLATE` DELETES a phase from the prefill chunk so its cost is read off by
    // subtraction. The output is wrong by construction — a stopwatch, not a lane — and the equivalence
    // locks never see it, because the default is "none".
    //
    // It exists because I asserted where prefill time goes and then had to retract it: that claim
    // scaled a 3060 B=1 GDN microbench onto an H100 B=32 whole-model measurement, where the 3060 GDN
    // figure is 16x LARGER than an entire H100 layer. Attribution by deletion is what produced every
    // trustworthy decode decision here; prefill never got the same treatment.
    let pablate = std::env::var("LOGOS_Q38B_PABLATE").unwrap_or_else(|_| "none".to_string());
    let (ab_gdn, ab_ffn) = (pablate == "gdn", pablate == "ffn");
    // The remaining phases, so "everything else" stops being 41% of prefill in one unattributed bucket.
    // `attn` is the whole per-token attention loop (5 launches x PT x 16 layers a chunk); `conv` is the
    // per-token depthwise conv plus the gated norm; `norm` is the residual-add RMSNorm.
    let (ab_attn, ab_conv, ab_norm) = (pablate == "attn", pablate == "conv", pablate == "norm");

    let mut pstep = String::new();
    if prefill_t > 1 {
        // ★ Prefill uses the ROWS-IN-M tile. `bq_gemm_tc1` reads the weight tile once per TC_TJ rows,
        // so at TC_TJ=32 a chunk of B*T rows costs B*T/32 passes and chunking buys NOTHING (measured:
        // 1.00x at B=32). `bq_gemm_tcm` reads it once per MI=256 rows. It is single-pass only, which
        // the fast lane is; a two-pass prefill would have to keep the other tile.
        // The rows-in-M tile is worth its 128-row height only when the chunk HAS rows to fill it.
        // A speculative verify is a chunk of 2, and lowering that through a 128-row tile measured
        // 84 ms against decode's 16 ms for the same weight read -- 64 of every 66 rows masked, and
        // the whole speculative gain eaten before it started. Below a full tile, take the decode
        // lowering over BT rows instead; it is the one tuned for a handful of rows.
        let bt_rows = batch * prefill_t;
        let mi_tcm = 16 * (8 / (64 / (8 * 2))) * 4; // MI_TCM from MT_NT=64, MT_WN=2, MT_WM=4
        let pg = |y: &str, w: &str, xh: &str, xs: &str, k: usize, n: usize, ys: usize| -> String {
            if hl || bt_rows < mi_tcm {
                return gemv_rows(y, w, xh, xs, k, n, ys, "BT");
            }
            format!(
                "bq_gemm_tcm<<<dim3(({n}+MT_NT-1)/MT_NT,(BT+MI_TCM-1)/MI_TCM),256,0,s>>>\
({y},{w}_q,{w}_s,{xh},{xs},{k},{n},BT,{ys});"
            )
        };
        for l in 0..cfg.layers {
            let entry = if l == 0 {
                "bq_rmsnorm_q8f_hl<<<BT,RMSN_NT,0,s>>>(x,aqE,aqEl,asE,x_f32,an0_f,E,EPSV);"
                    .to_string()
            } else {
                if ab_norm {
                    String::new()
                } else {
                    format!("bq_add_rmsnorm_q8f_hl<<<BT,RMSN_NT,0,s>>>(x,tmp,aqE,aqEl,asE,x_f32,an{l}_f,E,EPSV);")
                }
            };
            pstep.push_str(&format!("  {entry}\n  bq_trace_rows(s,x_f32,BT,E);\n"));
            if cfg.is_attn(l) {
                pstep.push_str(&format!(
                    "  {g}\n\
                     \x20 {abattn}for(int t=0;t<PT*{attn_on};++t){{ const long po=(long)t*B*PMAX;\n\
                     \x20   bq_pos_set<<<Gs(B),256,0,s>>>(d_pos,d_pbase,t,B);\n\
                     \x20   bq_kv_store<<<dim3(Gs({akv}*2),B),256,0,s>>>(kc{l},vc{l},d_pos,{akv},pbuf+po+{two_aq},PMAX);\n\
                     \x20   bq_qk_norm_rope_part<<<dim3({h},B),128,0,s>>>(pbuf+po,d_zero,0,PMAX,Qn{l}_f,{h},{hd},{rot},ROPEV,EPSV);\n\
                     \x20   bq_qk_norm_rope_kv<<<dim3({kv},B),128,0,s>>>(kc{l},d_pos,{akv},MAXSEQ*{akv},Kn{l}_f,{kv},{hd},{rot},ROPEV,EPSV);\n\
                     \x20   bq_attn_launch(s,attnb,pbuf+po,PMAX,kc{l},vc{l},d_pos,{h},{kv},{hd},SCALEV,B);\n\
                     \x20   bq_gate_mul<<<dim3(Gs({aq}),B),256,0,s>>>(attnb,{aq},pbuf+po+{aq},PMAX,{aq});\n\
                     \x20   bq_quant_act_hl<<<dim3(Gs({aq}),B),256,0,s>>>(attnb,aqQ+(long)t*B*{aq},aqQl+(long)t*B*{aq},asQ+(long)t*B*({aq}/32),{aq});\n\
                     \x20 }}\n\
                     \x20 {gw}\n",
                    abattn = "", attn_on = if ab_attn { 0 } else { 1 },
                    g = dg_site(1, "pbuf", &format!("qgkv{l}"), "x_f32", d, attnw, projw.max(attnw),
                                &pg("pbuf", &format!("qgkv{l}"), "aqE", "asE", d, attnw, projw.max(attnw))),
                    gw = pg("tmp", &format!("Wo{l}"), "aqQ", "asQ", aq, d, d),
                    two_aq = 2 * aq, h = cfg.h, kv = cfg.kv, hd = cfg.hd, rot = cfg.rope_dim));
            } else {
                pstep.push_str(&format!(
                    "  {g}\n\
                     \x20 bq_gemv_f32w<<<dim3(({two_nv}+(8/F32W_NW)-1)/(8/F32W_NW),BT),256,0,s>>>(gbuf,ba{l}_f,x_f32,E,{two_nv},BT);\n\
                     \x20 for(int t=0;t<PT*{conv_on};++t)\n\
                     \x20   bq_conv_silu_l2<<<dim3({convc}/{ghd},B),{ghd},0,s>>>(pbuf+(long)t*B*PMAX,PMAX,Wc{l}_f,cs{l},cbuf+(long)t*B*{convc},{convc},{taps},{two_nk},EPSV);\n\
                     \x20 {abgates}{gdncall}\n\
                     \x20 bq_gate_norm_hl<<<dim3({nv},BT),{ghd},0,s>>>(pbuf+{qkv_off},Gn{l}_f,obuf,gnbuf,aqQ,aqQl,asQ,{ghd},{nv},PMAX,EPSV);\n\
                     \x20 {gw}\n",
                    g = dg_site(1, "pbuf", &format!("qkvz{l}"), "x_f32", d, projw, projw.max(attnw),
                                &pg("pbuf", &format!("qkvz{l}"), "aqE", "asE", d, projw, projw.max(attnw))),
                    gw = pg("tmp", &format!("Wo{l}"), "aqQ", "asQ", vw, d, d),
                    conv_on = if ab_conv { 0 } else { 1 },
                    abgates = if ab_gdn { String::new() } else {
                        format!("bq_gdn_gates<<<dim3(Gs({nv}),BT),256,0,s>>>(gbuf,Dt{l}_f,Al{l}_f,betab,decayb,{nv});\n\x20 ")
                    },
                    gdncall = if ab_gdn { String::new() } else if mtp {
                        format!("bq_gdn_chunk_mid<<<dim3({nv},BQ_GDN_CHUNK_GY({ghd}),B),128,0,s>>>(cbuf,cbuf+{qk},cbuf+{two_qk},betab,decayb,st{l},stmid{l},obuf,{ghd},{nk},{nv},{convc},PT,B,MID);",
                                nv = nv, ghd = ghd, qk = qk, two_qk = 2 * qk, l = l, nk = nk, convc = convc)
                    } else {
                        format!("bq_gdn_chunk<<<dim3({nv},BQ_GDN_CHUNK_GY({ghd}),B),128,0,s>>>(cbuf,cbuf+{qk},cbuf+{two_qk},betab,decayb,st{l},obuf,{ghd},{nk},{nv},{convc},PT,B);",
                                nv = nv, ghd = ghd, qk = qk, two_qk = 2 * qk, l = l, nk = nk, convc = convc)
                    },
                    two_nv = 2 * nv, two_nk = 2 * nk, qkv_off = 2 * qk + vw));
            }
            pstep.push_str(&format!(
                "  bq_add_rmsnorm_q8f_hl<<<BT,RMSN_NT,0,s>>>(x,tmp,ffQ,ffQl,ffS,x2n,fn{l}_f,E,EPSV);\n\
                 \x20 {g_gu}\n\
                 \x20 {g_wd}\n\
                 \x20 {resid}",
                // ★ THE DEEPGEMM SITE. Both lanes are emitted; the BUILD picks one, so the default
                // program is unchanged and the lane needs no emitter flag. Measured at this exact
                // shape: 631 TFLOPS against our tile's 146 — 4.32x.
                //
                // ACCURACY, stated correctly. The standalone bench's rel_L2 of 1.07e-4 measured the
                // kernel's ACCUMULATION against a reference recomputed from the fp8 bytes it was
                // already given; it never contained the int8 -> fp8 quantisation loss. End to end the
                // lane is roughly TWICE as lossy as the int8 it replaces — e4m3 carries 3 mantissa
                // bits against int8's 8, blocked per 128 instead of per 32 — which is why its token
                // stream diverges from ours through 64 layers of a gated recurrence. That is a real
                // price, it is the same price vLLM's fp8 pays, and it is pinned by
                // `the_deepgemm_fp8_lane_costs_about_twice_the_quantisation_error_of_the_int8_lane`.
                // The A/CD descriptors are built per call because `PTv` is a runtime chunk length;
                // prefill_chunk is not graph-captured, so host code here is fine.
                g_gu = if ab_ffn { String::new() } else { dg_site(
                    0, "ffbuf", &format!("gu{l}"), "x2n", d, 2 * dff, 2 * dff,
                    &pg("ffbuf", &format!("gu{l}"), "ffQ", "ffS", d, 2 * dff, 2 * dff)) },
                g_wd = if ab_ffn { String::new() } else { dg_site_p(
                    2, "tmp", &format!("Wd{l}"),
                    &format!("bq_silu_had_fp8_dg<<<1024,256,0,s>>>(dgA,dgAsf,ffbuf,_m,{dff},_ma);"),
                    dff, d, d,
                    &format!("bq_silu_had_q8_hl<<<dim3(Gs({dff}),BT),256,0,s>>>(ffbuf,hqQ,hqQl,hqS,{dff});\n\x20 {}",
                             pg("tmp", &format!("Wd{l}"), "hqQ", "hqS", dff, d, d))) },
                resid = if l + 1 == cfg.layers { "bq_add<<<dim3(Gs(E),BT),256,0,s>>>(x,tmp,E);\n" } else { "\n" }));
        }
    }

    // ── THE DRAFT HEAD ────────────────────────────────────────────────────────────────────────────
    // blk.{layers} conditioned on [norm(embedding of the token just produced) ++ norm(residual
    // stream)] through a 2E -> E adapter, then an ordinary gated-attention layer + FFN + the model's
    // own output head. It runs on the MAIN model's residual stream `x`, so it must be emitted to read
    // x BEFORE the verify pass overwrites it, and it keeps its OWN KV cache: the draft attends to its
    // own single layer's history, not the model's.
    let mtpfn = if !mtp {
        String::new()
    } else {
        let l = cfg.layers;
        let two_d = 2 * d;
        format!(
"static void mtp_draft(cudaStream_t s){{\n\
\x20 bq_embed<<<dim3(Gs(E),B),256,0,s>>>(membed,emb_q,emb_s,d_tok,E);\n\
\x20 bq_mtp_cat<<<dim3(2,B),256,0,s>>>(mcat,membed,x,enorm_f,hnorm_f,E,EPSV);\n\
\x20 bq_quant_act_hl<<<dim3(Gs({two_d}),B),256,0,s>>>(mcat,maq,maql,mas,{two_d});\n\
\x20 {g_eh}\n\
\x20 bq_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(mx,maq,maql,mas,mf32,an{l}_f,E,EPSV);\n\
\x20 {g_qgkv}\n\
\x20 bq_kv_store<<<dim3(Gs({akv}*2),B),256,0,s>>>(mkc,mvc,d_pos,{akv},pbuf+{two_aq},PMAX);\n\
\x20 bq_qk_norm_rope_part<<<dim3({h},B),128,0,s>>>(pbuf,d_zero,0,PMAX,Qn{l}_f,{h},{hd},{rot},ROPEV,EPSV);\n\
\x20 bq_qk_norm_rope_part<<<dim3({kv},B),128,0,s>>>(mkc,d_pos,{akv},MAXSEQ*{akv},Kn{l}_f,{kv},{hd},{rot},ROPEV,EPSV);\n\
\x20 bq_attn_launch(s,attnb,pbuf,PMAX,mkc,mvc,d_pos,{h},{kv},{hd},SCALEV,B);\n\
\x20 bq_gate_mul<<<dim3(Gs({aq}),B),256,0,s>>>(attnb,{aq},pbuf+{aq},PMAX,{aq});\n\
\x20 bq_quant_act_hl<<<dim3(Gs({aq}),B),256,0,s>>>(attnb,maq,maql,mas,{aq});\n\
\x20 {g_wo}\n\
\x20 bq_add_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(mx,mtmp,maq,maql,mas,m2n,fn{l}_f,E,EPSV);\n\
\x20 {g_gu}\n\
\x20 bq_silu_had_q8_hl<<<dim3(Gs({dff}),B),256,0,s>>>(ffbuf,hqQ,hqQl,hqS,{dff});\n\
\x20 {g_wd}\n\
\x20 bq_add<<<dim3(Gs(E),B),256,0,s>>>(mx,mtmp,E);\n\
\x20 bq_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(mx,maq,maql,mas,mf32,shnorm_f,E,EPSV);\n\
\x20 {g_head}\n\
\x20 bq_argmax_rows<<<B,256,0,s>>>(dlogits,VOCAB,d_draft);\n\
}}\n\n",
            g_eh   = gemv("mx", "ehproj", "maq", "mas", two_d, d, d),
            g_qgkv = gemv("pbuf", &format!("qgkv{l}"), "maq", "mas", d, attnw, projw.max(attnw)),
            g_wo   = gemv("mtmp", &format!("Wo{l}"), "maq", "mas", aq, d, d),
            g_gu   = gemv("ffbuf", &format!("gu{l}"), "maq", "mas", d, 2 * dff, 2 * dff),
            g_wd   = gemv("mtmp", &format!("Wd{l}"), "hqQ", "hqS", dff, d, d),
            g_head = gemv("dlogits", "Wout", "maq", "mas", d, cfg.vocab, cfg.vocab),
            two_aq = 2 * aq, h = cfg.h, kv = cfg.kv, hd = cfg.hd, rot = cfg.rope_dim)
        + &format!(
"static void mtp_verify(cudaStream_t s){{\n\
\x20 prefill_chunk(s,1,2);   /* a verify is draft + current: two tokens, never PT */\n\
\x20 bq_rmsnorm_q8f_hl<<<BT,RMSN_NT,0,s>>>(x,aqE,aqEl,asE,x_f32,on_f,E,EPSV);\n\
\x20 {g_vhead}\n\
\x20 bq_argmax_rows<<<BT,256,0,s>>>(vlogits,VOCAB,d_verify);\n\
}}\n\n",
            g_vhead = gemv_rows("vlogits", "Wout", "aqE", "asE", d, cfg.vocab, cfg.vocab, "BT"))
        + &format!(
"// One speculative step: draft a token, check it against the model in the SAME pass that would have
// produced one token anyway, and keep 1 or 2 depending on whether the draft held. Every branch is on
// the device -- `bq_mtp_accept` writes the next token and the next position itself -- so this is a
// fixed kernel sequence and captures as a graph exactly like decode_step.
static void mtp_step(cudaStream_t s){{\n\
\x20 cudaMemcpyAsync(d_pbase,d_pos,(size_t)B*4,cudaMemcpyDeviceToDevice,s);\n\
\x20 mtp_draft(s);\n\
\x20 bq_mtp_pack<<<Gs(B),256,0,s>>>(d_tok,d_draft,B);\n\
\x20 mtp_verify(s);\n\
\x20 bq_mtp_accept<<<Gs(B),256,0,s>>>(d_verify,d_draft,d_tok,d_pos,d_acc,B);\n\
\x20 bq_mtp_pick_hidden<<<dim3(32,B),256,0,s>>>(x,d_acc,B,E);\n\
{roll}\
}}\n\n", roll = mtp_roll)
    };

    // Tokens per pass over the weights is 1 + accept, so the figure to compare is TOKENS, not steps.
    // Captured as a graph like decode: every branch lives on the device, so the sequence is fixed.
    let mtploop = if !mtp {
        String::new()
    } else {
        String::from(
            r#"  {
    cudaGraph_t mg; cudaGraphExec_t mge;
    // Rewind to the start of the cache first. Timing runs 64 speculative steps and each consumes up
    // to TWO positions, so starting where generation stopped walks straight off the end of MAXSEQ --
    // which is an illegal access inside a graph replay, well away from the line that caused it.
    for(int b=0;b<B;++b) hpos[b]=0;
    cudaMemcpy(d_pos,hpos,(size_t)B*4,cudaMemcpyHostToDevice);
    cudaStreamSynchronize(s);
    cudaMemset(d_acc,0,(size_t)(B+1)*4);
    mtp_step(s); cudaStreamSynchronize(s);
    if(cudaGetLastError()!=cudaSuccess){ fprintf(stderr,"MTP WARMUP: %s\n",cudaGetErrorString(cudaGetLastError())); return 1; }
    if(cudaStreamBeginCapture(s,cudaStreamCaptureModeGlobal)!=cudaSuccess){ fprintf(stderr,"mtp capture\n"); return 1; }
    mtp_step(s);
    if(cudaStreamEndCapture(s,&mg)!=cudaSuccess){ fprintf(stderr,"mtp endcapture\n"); return 1; }
    if(cudaGraphInstantiate(&mge,mg,0)!=cudaSuccess){ fprintf(stderr,"mtp inst\n"); return 1; }
    const int MSTEPS=64;
    cudaMemset(d_acc,0,(size_t)(B+1)*4);
    cudaEvent_t m0,m1; cudaEventCreate(&m0); cudaEventCreate(&m1);
    cudaEventRecord(m0,s);
    for(int i=0;i<MSTEPS;++i) cudaGraphLaunch(mge,s);
    cudaEventRecord(m1,s);
    if(cudaStreamSynchronize(s)!=cudaSuccess){ fprintf(stderr,"MTP RUN: %s\n",cudaGetErrorString(cudaGetLastError())); return 1; }
    float mms=0; cudaEventElapsedTime(&mms,m0,m1);
    int nacc=0; cudaMemcpy(&nacc,d_acc+B,4,cudaMemcpyDeviceToHost);
    double toks=(double)MSTEPS*B+(double)nacc;
    fprintf(stderr,"MTP B %d  ACCEPT %.3f  TOK_S %.2f  (%.3f ms/step, %.2f tok/step/seq)\n",
            B, (double)nacc/((double)MSTEPS*B), toks*1000.0/mms, mms/MSTEPS, toks/((double)MSTEPS*B));
  }
"#,
        )
    };

    // Emitted BEFORE the format! below, because `skp_alloc` reads the split-K high-water mark and
    // format! evaluates its arguments in source order. The output head is a matmul site like any
    // other and at batch 16 it is the WIDEST splitter in the program (2 x 151936 against the
    // largest projection's 8 x 34816), so sizing the buffer before it was emitted under-allocated
    // by 100 KB and corrupted every sequence but the first. `BATCH_AGREEMENT` caught it: 0/16.
    let head_launch = if use_wide {
        "bq_gemv_wide<<<dim3((VOCAB+MV_RB-1)/MV_RB,1),MV_NW*32,0,s>>>(logits,Wout_q,Wout_s,aqE,aqEl,asE,E,VOCAB,B,VOCAB,(float*)0,1);".to_string()
    } else if use_tc {
        // The head is emitted through its own path rather than the `gemv` closure, which is why it
        // is the site that gets left behind. VOCAB is wide enough that the split rule returns 1,
        // but it is computed rather than assumed so a narrower vocabulary still fills the machine.
        let sp = tc_split(d, cfg.vocab);
        let blocks = cfg.vocab.div_ceil(tc_ti);
        // Same two instantiations as the projections: hi+lo on the golden lane, hi alone on the
        // fast one. The head is the single widest matmul in the step (VOCAB = 248320), so leaving
        // it on the two-pass tile would have kept half the saving.
        let (tck, aqops) = if hl {
            ("bq_gemm_tc", "aqE,aqEl")
        } else {
            ("bq_gemm_tc1", "aqE")
        };
        if sp > 1 {
            tc_part_max.set(tc_part_max.get().max(sp * cfg.vocab));
            format!("{tck}<<<dim3({blocks},{sp}),256,0,s>>>(logits,Wout_q,Wout_s,{aqops},asE,E,VOCAB,B,VOCAB,skp,{sp});\n\
                         \x20 bq_sk_reduce<<<Gs(VOCAB*B),256,0,s>>>(logits,skp,VOCAB,B,VOCAB,{sp});")
        } else {
            format!("{tck}<<<dim3({blocks},1),256,0,s>>>(logits,Wout_q,Wout_s,{aqops},asE,E,VOCAB,B,VOCAB,(float*)0,1);")
        }
    } else if use_mma {
        "bq_gemm_mma<<<dim3((VOCAB+TBN-1)/TBN,(B+TBM-1)/TBM),256,(size_t)(TBM+TBN)*(KSTEP+4),s>>>(logits,Wout_q,Wout_s,aqE,asE,E,VOCAB,B,VOCAB);".to_string()
    } else if use_mmq {
        "bq_gemm_mmq<<<(VOCAB+((256/(MMQ_TJ/MMQ_RJ))*MMQ_RI)-1)/((256/(MMQ_TJ/MMQ_RJ))*MMQ_RI),256,0,s>>>(logits,Wout_q,Wout_s,aqE,aqEl,asE,E,VOCAB,B,VOCAB);".to_string()
    } else if use_staged && hl {
        format!("bq_gemv_hl_smem<<<(VOCAB+7)/8,256,(size_t)(2*{batch}*512+{batch}*16*4),s>>>(logits,Wout_q,Wout_s,aqE,aqEl,asE,E,VOCAB,B,VOCAB);")
    } else if use_staged {
        format!("bq_gemv_h_smem<<<(VOCAB+7)/8,256,(size_t)({batch}*512+{batch}*16*4),s>>>(logits,Wout_q,Wout_s,aqE,asE,E,VOCAB,B,VOCAB);")
    } else if hl {
        "bq_gemv_hl<<<dim3(Gw(VOCAB),".to_string()
            + &btiles.to_string()
            + "),256,0,s>>>(logits,Wout_q,Wout_s,aqE,aqEl,asE,E,VOCAB,B,VOCAB);"
    } else {
        "bq_gemv_h<<<dim3(Gw(VOCAB),".to_string()
            + &btiles.to_string()
            + "),256,0,s>>>(logits,Wout_q,Wout_s,aqE,asE,E,VOCAB,B,VOCAB);"
    };

    let prov = super::provenance::header(super::provenance::Provenance::HandWritten {
        model: "Qwen3.8-27B",
        lane: "batched decode",
    });
    format!(
"{prov}// Qwen3.8-27B BATCHED decode.
// {batch} sequences share ONE pass over the weights; only the recurrence state, conv rings and KV caches
// are per-sequence. State cost: {state_mb:.1} MB per sequence at MAXSEQ={maxseq}.
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cmath>
#include <cuda_runtime.h>
#include <cuda_fp16.h>
#include <chrono>
// Only the macros the KERNELS themselves reference may be defined before them. `E` and `B` are parameter
// names in the shared kernel set and in the batched GEMVs respectively, so defining them here would rewrite
// `int E` into `int 5120` and `int B` into `int 8` — which nvcc reports as a missing parenthesis several
// hundred lines later, in an unrelated function.
#define BTG {btg}
#define BMAX {batch}
#define MAXSEQ {maxseq}
// MMQ tile: sequence tile rounded up to MMQ_RJ, K staged per barrier round, and the accumulator type.
// `double` is measured free on a datacentre part (fp64 half-rate) and 10x more accurate than float;
// on a consumer card fp64 is 1/32 rate, which is why LOGOS_Q38B_MMQ_ACC exists.
#define MMQ_TJ {mmq_tj}
#define MMQ_RI 2
#define MMQ_RJ 2
#define MMQ_KC 256
#define MMQ_ACC {mmq_acc}
// Tensor-core tile: sequence tile (a power of two so the warp -> sub-tile map is a bijection),
// 16-row sub-tiles per warp, and K staged per barrier round. All three measured, not assumed.
#define TC_TJ {tc_tj}
{q4_def}#define TC_WM {tc_wm}
#define BQ_TC_LANES {tc_lanes}
#define TC_KC {tc_kc}
{tc_occ_def}
#define TC_STAGES {tc_stages}
#define PT {prefill_t}
#define BT (B*PT)
// Rows-in-M tile (prefill): MI sequence rows per block against MT_NT channels.
#define MT_WM 4
#define MT_NT 64
#define MT_WN 2
#define MT_KC 64
#define MI_TCM (16*(8/(MT_NT/(8*MT_WN)))*MT_WM)
// Row-parallel GEMV: sequences in registers, output rows per block, warps reducing K together.
#define MV_TJ {mv_tj}
#define MV_RB {mv_rb}
#define MV_NW {mv_nw}
#define RMSN_NT {rmsn_nt}
#define NORM_G {norm_g}
{gdn_k}
{q38_k}
{extra_k}
{bat_k}
#define B {batch}
#define PMAX {pmax}
#define E {d}
#define VOCAB {vocab}
#define EPSV {eps:e}f
#define ROPEV {rope:e}f
#define SCALEV {scale:e}f
static float *x,*tmp,*x2n,*x_f32,*pbuf,*cbuf,*obuf,*gnbuf,*gbuf,*betab,*decayb,*attnb,*ffbuf,*logits;
static signed char *aqE,*aqQ,*ffQ,*hqQ,*aqEl,*aqQl,*ffQl,*hqQl; static float *asE,*asQ,*ffS,*hqS;
static int *d_pos,*d_tok,*d_zero,*d_pbase;\nstatic int hpos[B];
static float* skp; static float* normp;
{decls}#ifdef BQ_ENABLE_DG
{dg_decls}#else
static const int bq_dg_on[3] = {{0,0,0}};
#endif
{state_decl}
static inline int Gw(int n){{ return (n*32+255)/256; }}
static inline int Gs(int n){{ return (n+255)/256; }}
#define LDQ(v,QN,SN,QP,SP) do{{ \\
  signed char* hq=(signed char*)malloc((size_t)QN); float* hs=(float*)malloc((size_t)SN*4); \\
  FILE* f=fopen(QP,\"rb\"); if(!f||fread(hq,1,(size_t)QN,f)!=(size_t)QN){{fprintf(stderr,\"load %s\\n\",QP);return 1;}} fclose(f); \\
  f=fopen(SP,\"rb\"); if(!f||fread(hs,4,(size_t)SN,f)!=(size_t)SN){{fprintf(stderr,\"load %s\\n\",SP);return 1;}} fclose(f); \\
  __half* hh=(__half*)malloc((size_t)(SN)*2); for(long i=0;i<(long)(SN);++i) hh[i]=__float2half(hs[i]); \\
  cudaMalloc(&v##_q,(size_t)QN); cudaMemcpy(v##_q,hq,(size_t)QN,cudaMemcpyHostToDevice); \\
  cudaMalloc(&v##_s,(size_t)(SN)*2); cudaMemcpy(v##_s,hh,(size_t)(SN)*2,cudaMemcpyHostToDevice); \\
  free(hq); free(hs); free(hh); }}while(0)
#define LDF(v,N,P) do{{ \\
  float* h=(float*)malloc((size_t)N*4); FILE* f=fopen(P,\"rb\"); \\
  if(!f||fread(h,4,(size_t)N,f)!=(size_t)N){{fprintf(stderr,\"load %s\\n\",P);return 1;}} fclose(f); \\
  cudaMalloc(&v##_f,(size_t)N*4); cudaMemcpy(v##_f,h,(size_t)N*4,cudaMemcpyHostToDevice); free(h); }}while(0)

{pfn}{mtpfn}static void decode_step(cudaStream_t s){{
  bq_embed<<<dim3(Gs(E),B),256,0,s>>>(x,emb_q,emb_s,d_tok,E);
{step}\x20 bq_rmsnorm_q8f_hl<<<B,RMSN_NT,0,s>>>(x,aqE,aqEl,asE,x_f32,on_f,E,EPSV);
  {head}
  bq_argmax_adv<<<B,256,0,s>>>(logits,VOCAB,d_tok,d_pos);
}}

int main(){{
  cudaMalloc(&x,(size_t)BT*E*4); cudaMalloc(&tmp,(size_t)BT*E*4); cudaMalloc(&x2n,(size_t)BT*E*4);
  cudaMalloc(&x_f32,(size_t)BT*E*4);
  cudaMalloc(&pbuf,(size_t)BT*{pmax}*4); cudaMalloc(&cbuf,(size_t)BT*{convc}*4);
  cudaMalloc(&obuf,(size_t)BT*{vw}*4); cudaMalloc(&gnbuf,(size_t)BT*{vw}*4);
  cudaMalloc(&gbuf,(size_t)BT*{two_nv}*4); cudaMalloc(&betab,(size_t)BT*{nv}*4); cudaMalloc(&decayb,(size_t)BT*{nv}*4);
  cudaMalloc(&attnb,(size_t)B*{aq}*4); cudaMalloc(&ffbuf,(size_t)BT*{two_dff}*4);
  cudaMalloc(&logits,(size_t)B*VOCAB*4);
{tracopen}
{mtpalloc}
  cudaMalloc(&aqE,(size_t)BT*E); cudaMalloc(&aqEl,(size_t)BT*E); cudaMalloc(&asE,(size_t)BT*(E/32)*4);
  cudaMalloc(&aqQ,(size_t)BT*{amax}); cudaMalloc(&aqQl,(size_t)BT*{amax}); cudaMalloc(&asQ,(size_t)BT*({amax}/32)*4);
  cudaMalloc(&ffQ,(size_t)BT*E); cudaMalloc(&ffQl,(size_t)BT*E); cudaMalloc(&ffS,(size_t)BT*(E/32)*4);
  cudaMalloc(&hqQ,(size_t)BT*{dff}); cudaMalloc(&hqQl,(size_t)BT*{dff}); cudaMalloc(&hqS,(size_t)BT*({dff}/32)*4);
  cudaMalloc(&d_pos,(size_t)B*4); cudaMalloc(&d_tok,(size_t)BT*4);\n  // B ints, not one: `bq_qk_norm_rope_part` is launched over dim3(heads,B) and reads d_pos[bb]\n  // for every sequence. A 4-byte d_zero was an OUT-OF-BOUNDS read for bb>0 from the moment d_pos\n  // became per-sequence -- silent, and dependent on whatever allocation happened to sit next to\n  // it, which is why it only changed the answer once BT grew and moved the heap around.\n  cudaMalloc(&d_zero,(size_t)B*4);
  cudaMalloc(&d_pbase,(size_t)B*4);
{skp_alloc}{norm_alloc}
{state_alloc}{loads}#ifdef BQ_ENABLE_DG
  // Convert the gate/up weights to fp8 e4m3 with per-128 scales and build their TMA descriptors ONCE.
  // The Q8_0 copies stay: only the gate/up sites are routed, and the rest of the model still reads them.
{dg_loads}#endif

  {{ cudaError_t e=cudaGetLastError(); if(e!=cudaSuccess){{
       fprintf(stderr,\"CUDA ERROR after weight load: %s\\n\",cudaGetErrorString(e)); return 1; }} }}
  {{ for(int b=0;b<B;++b) hpos[b]=0; cudaMemcpy(d_zero,hpos,(size_t)B*4,cudaMemcpyHostToDevice);\n     for(int b=0;b<B;++b) hpos[b]=0; cudaMemcpy(d_pos,hpos,(size_t)B*4,cudaMemcpyHostToDevice); }}
  cudaStream_t s; cudaStreamCreate(&s);
  static const int PROMPT[] = {{{prompt_list}}}; const int NP = sizeof(PROMPT)/sizeof(int);
  static int htok[B]; static int htokc[BT];
  // ── TTFT ────────────────────────────────────────────────────────────────────────────────────
  // Prefill here is the T=1 decode graph replayed ONCE PER PROMPT TOKEN, so a 512-token prompt pays
  // 512 whole forwards before the first token exists. vLLM prefills a chunk in one batched pass and
  // reports median_ttft_ms per request; this timer is what makes the two comparable -- and what makes
  // the deficit a measured number rather than an argument.
  auto tt0 = std::chrono::steady_clock::now();
  int i0 = 0;
#if PT > 1
  // Whole chunks first: PT prompt tokens share ONE pass over the weights. Rows are token-major
  // (t*B + b), which is why token t's slice is a contiguous base-pointer offset for the recurrence.
  // NP-1, not NP: `prefill_chunk` deliberately has no LM head (it is the widest matmul in the
  // program and prefill needs the hidden state, not logits), so the LAST prompt token must go through
  // `decode_step`, which produces the logits the first generated token comes from AND advances d_pos
  // via `bq_argmax_adv`. Consuming the whole prompt in chunks leaves no logits and d_pos one short --
  // which is exactly why the first gate run produced a different token stream.
  for(; i0 + PT <= NP-1; i0 += PT){{
    for(int b=0;b<B;++b) hpos[b]=i0; cudaMemcpy(d_pbase,hpos,(size_t)B*4,cudaMemcpyHostToDevice);
    for(int t=0;t<PT;++t) for(int b=0;b<B;++b) htokc[t*B+b]=PROMPT[i0+t];
    cudaMemcpy(d_tok,htokc,(size_t)BT*4,cudaMemcpyHostToDevice);
    prefill_chunk(s,0,PT);
    {{ cudaError_t e=cudaGetLastError(); if(e!=cudaSuccess){{
         fprintf(stderr,\"CUDA LAUNCH ERROR at prefill chunk %d: %s\\n\",i0,cudaGetErrorString(e)); return 1; }} }}
    if(cudaStreamSynchronize(s)!=cudaSuccess){{
      fprintf(stderr,\"CUDA EXEC ERROR at prefill chunk %d: %s\\n\",i0,cudaGetErrorString(cudaGetLastError())); return 1; }}
    // the chunk consumed PT positions; decode resumes from there
    for(int b=0;b<B;++b) hpos[b]=i0+PT-1; cudaMemcpy(d_pos,hpos,(size_t)B*4,cudaMemcpyHostToDevice);
  }}
#endif
  for(int i=i0;i<NP;++i){{
    for(int b=0;b<B;++b) hpos[b]=i; cudaMemcpy(d_pos,hpos,(size_t)B*4,cudaMemcpyHostToDevice);
    for(int b=0;b<B;++b) htok[b]=PROMPT[i];
    cudaMemcpy(d_tok,htok,(size_t)B*4,cudaMemcpyHostToDevice);
    decode_step(s);
    {{ cudaError_t e=cudaGetLastError(); if(e!=cudaSuccess){{
         fprintf(stderr,\"CUDA LAUNCH ERROR at prefill %d: %s\\n\",i,cudaGetErrorString(e)); return 1; }} }}
    if(cudaStreamSynchronize(s)!=cudaSuccess){{
      fprintf(stderr,\"CUDA EXEC ERROR at prefill %d: %s\\n\",i,cudaGetErrorString(cudaGetLastError())); return 1; }}
  }}
  {{ auto tt1 = std::chrono::steady_clock::now();
     double pf = std::chrono::duration<double,std::milli>(tt1-tt0).count();
     // The last prefill step already produced the first generated token, so time-to-first-token is the
     // prefill span plus the copy that makes it visible to the host -- measured, not modelled.
     static int hfirst[B]; cudaMemcpy(hfirst,d_tok,(size_t)B*4,cudaMemcpyDeviceToHost);
     auto tt2 = std::chrono::steady_clock::now();
     double ttft = std::chrono::duration<double,std::milli>(tt2-tt0).count();
     fprintf(stderr,\"PREFILL_MS %.3f  (%d prompt tokens, %.3f ms/token)\\n\", pf, NP, pf/(NP?NP:1));
     fprintf(stderr,\"TTFT_MS %.3f  (batch %d, first token %d)\\n\", ttft, B, hfirst[0]); }}
  {{ // the logits the prompt produced, before anything is generated -- a sanity read, not a metric
    static float hl0[16]; cudaMemcpy(hl0,logits,16*4,cudaMemcpyDeviceToHost);
    fprintf(stderr,\"PREFILL_LOGITS[0..3] %.4f %.4f %.4f %.4f\\n\",hl0[0],hl0[1],hl0[2],hl0[3]); }}
  cudaGraph_t g; cudaGraphExec_t ge;
  if(cudaStreamBeginCapture(s,cudaStreamCaptureModeGlobal)!=cudaSuccess){{fprintf(stderr,\"capture\\n\");return 1;}}
  decode_step(s);
  if(cudaStreamEndCapture(s,&g)!=cudaSuccess){{fprintf(stderr,\"endcapture\\n\");return 1;}}
  if(cudaGraphInstantiate(&ge,g,0)!=cudaSuccess){{fprintf(stderr,\"inst\\n\");return 1;}}
  cudaEvent_t t0,t1; cudaEventCreate(&t0); cudaEventCreate(&t1);
  cudaMemcpy(htok,d_tok,(size_t)B*4,cudaMemcpyDeviceToHost);
{mtptok}  printf(\"SEQ0_TOKENS: %d\", htok[0]);
  static int rows[B][{n_gen}]; for(int b=0;b<B;++b) rows[b][0]=htok[b];
  for(int i=1;i<{n_gen};++i){{
    cudaGraphLaunch(ge,s); cudaStreamSynchronize(s);
    cudaMemcpy(htok,d_tok,(size_t)B*4,cudaMemcpyDeviceToHost);
    printf(\" %d\", htok[0]);
    for(int b=0;b<B;++b) rows[b][i]=htok[b];
  }}
  printf(\"\\n\");
  int diverged=0;
  for(int b=1;b<B;++b) for(int i=0;i<{n_gen};++i) if(rows[b][i]!=rows[0][i]) diverged++;
  printf(\"BATCH_AGREEMENT: %d/%d sequences identical to sequence 0 (%d token mismatches)\\n\",
         diverged?0:B, B, diverged);
{mtploop}  {{ float ms=0; for(int b=0;b<B;++b) hpos[b]=0;\n     cudaMemcpy(d_pos,hpos,(size_t)B*4,cudaMemcpyHostToDevice); cudaStreamSynchronize(s);
     if(getenv(\"LOGOS_B_BISECT\")){{
       for(int i=0;i<128;++i){{ cudaGraphLaunch(ge,s);
         if(cudaStreamSynchronize(s)!=cudaSuccess){{ int hp=0; cudaMemcpy(&hp,d_pos,4,cudaMemcpyDeviceToHost);
           fprintf(stderr,\"FAULT at replay %d (d_pos=%d, MAXSEQ=%d): %s\\n\",i,hp,MAXSEQ,
                   cudaGetErrorString(cudaGetLastError())); return 1; }} }}
       fprintf(stderr,\"BISECT: all 128 replays clean\\n\"); }}
     /* ★ THE DISTRIBUTION, NOT JUST THE MEAN. vLLM reports median_itl_ms and p99_itl_ms, and its
        median ITL IS its step time — which is the fact that corrected this campaign's biggest
        misreading: comparing our pure-GPU decode throughput against vLLM's full serving number
        flattered us, and only the ITL comparison made the real 2.13x gap visible. A mean over a
        block cannot be compared to a median, and it hides the tail that serving is actually judged
        on. One event per step costs ~1 us against a ~33 ms step. */
     cudaEvent_t iev[129]; for(int i=0;i<=128;++i) cudaEventCreate(&iev[i]);
     cudaEventRecord(t0,s); cudaEventRecord(iev[0],s);
     for(int i=0;i<128;++i){{ cudaGraphLaunch(ge,s); cudaEventRecord(iev[i+1],s); }}
     cudaEventRecord(t1,s); cudaStreamSynchronize(s);
     cudaError_t te=cudaEventElapsedTime(&ms,t0,t1);
     if(te!=cudaSuccess||ms<=0.f){{ fprintf(stderr,\"TIMING FAILED: %s (ms=%.3f)\\n\",cudaGetErrorString(te),ms); return 1; }}
     fprintf(stderr,\"BATCH %d  PER_SEQ_TOK_S %.2f  AGGREGATE_TOK_S %.2f  (%.3f ms/step)\\n\",
             B, 128000.0/ms, 128000.0*B/ms, ms/128.0);
     {{ float itl[128]; int ok=1;
        for(int i=0;i<128;++i) if(cudaEventElapsedTime(&itl[i],iev[i],iev[i+1])!=cudaSuccess) ok=0;
        for(int i=0;i<=128;++i) cudaEventDestroy(iev[i]);
        if(ok){{
          for(int i=1;i<128;++i){{ float k=itl[i]; int j=i-1;
            while(j>=0&&itl[j]>k){{ itl[j+1]=itl[j]; --j; }} itl[j+1]=k; }}
          float mean=0.f; for(int i=0;i<128;++i) mean+=itl[i]; mean/=128.f;
          fprintf(stderr,\"ITL_MEAN_MS %.3f  ITL_MEDIAN_MS %.3f  ITL_P99_MS %.3f  ITL_MIN_MS %.3f\\n\",
                  mean, 0.5f*(itl[63]+itl[64]), itl[126], itl[0]);
        }} }} }}
  return 0;
}}
",
        btg = BTG, vocab = cfg.vocab, eps = cfg.rms_eps, rope = cfg.rope_theta,
        scale = 1.0 / (cfg.hd as f64).sqrt(),
        gdn_k = crate::codegen_cuda::gdn::GDN_KERNELS,
        q38_k = crate::codegen_cuda::gdn::QWEN38_KERNELS,
        extra_k = crate::codegen_cuda::qwen38::QWEN38_EXTRA_KERNELS,
        bat_k = QWEN38_BATCHED_KERNELS,
        // The output head is emitted through its own path, so every lowering decision has to be
        // repeated here. It is the WIDEST matmul in the model (VOCAB rows against E), which makes it
        // the most expensive site to leave on the L2-re-reading form — and it is exactly the site that
        // was missed when staging first landed, caught by the all-or-nothing assertion in
        // `qwen38_authored_lock` rather than by a benchmark.
        mtpfn = mtpfn, mtploop = mtploop,
        // Speculative decoding with EXACT verification is lossless in principle. In practice the
        // verify pass runs a TWO-row chunk where decode runs one, and a different reduction shape can
        // move a float. Whether the speculative stream IS the greedy stream is therefore a
        // measurement, not a theorem -- this mode emits it to be diffed from the same prefill state.
        mtptok = if !mtp { String::new() } else { String::from(r#"  if(getenv("LOGOS_MTP_TOKENS")){
    printf("MTP_TOKENS: %d", htok[0]);
    int produced=1, steps=0, acc=0; static int hv[2*BMAX], ha[BMAX+1];
    cudaEvent_t k0,k1; cudaEventCreate(&k0); cudaEventCreate(&k1); cudaEventRecord(k0,s);
    while(produced < 128){
      mtp_step(s);
      if(cudaStreamSynchronize(s)!=cudaSuccess){ fprintf(stderr,"MTP TOKENS: %s\\n",cudaGetErrorString(cudaGetLastError())); return 1; }
      cudaMemcpy(hv,d_verify,(size_t)2*B*4,cudaMemcpyDeviceToHost);
      cudaMemcpy(ha,d_acc,(size_t)B*4,cudaMemcpyDeviceToHost);
      ++steps; acc += ha[0];
      printf(" %d", hv[0]); ++produced;
      if(ha[0] && produced < 128){ printf(" %d", hv[B]); ++produced; }
    }
    printf("\\n");
    cudaEventRecord(k1,s); cudaStreamSynchronize(s);
    float kms=0; cudaEventElapsedTime(&kms,k0,k1);
    // Timed from the REAL post-prefill state, so the accept rate is the one this prompt
    // actually produces. A host sync per step is included, making the figure CONSERVATIVE.
    fprintf(stderr,"MTP_REAL B %d  ACCEPT %.3f  TOK_S %.2f  (%d steps, %d tokens, %.3f ms/step)\\n",
            B, (double)acc/steps, (double)produced*B*1000.0/kms, steps, produced, kms/steps);
    return 0;
  }
"#) },
        mtpalloc = if !mtp { String::new() } else {
            let mscratch = (2 * d).max(dff).max(aq);
            format!(
"\x20 cudaMalloc(&membed,(size_t)B*E*4); cudaMalloc(&mcat,(size_t)B*{two_d}*4);\n\
\x20 cudaMalloc(&mx,(size_t)B*E*4); cudaMalloc(&mtmp,(size_t)B*E*4);\n\
\x20 cudaMalloc(&m2n,(size_t)B*E*4); cudaMalloc(&mf32,(size_t)B*E*4);\n\
\x20 cudaMalloc(&dlogits,(size_t)B*VOCAB*4); cudaMalloc(&d_draft,(size_t)B*4);\n\
\x20 cudaMalloc(&mkc,(size_t)B*MAXSEQ*{akv}*4); cudaMalloc(&mvc,(size_t)B*MAXSEQ*{akv}*4);\n\
\x20 cudaMalloc(&maq,(size_t)B*{ms}); cudaMalloc(&maql,(size_t)B*{ms});\n\
\x20 cudaMalloc(&mas,(size_t)B*({ms}/32)*4);\n\
\x20 cudaMalloc(&vlogits,(size_t)BT*VOCAB*4); cudaMalloc(&d_verify,(size_t)BT*4);\n\
\x20 cudaMalloc(&d_acc,(size_t)(B+1)*4); cudaMemset(d_acc,0,(size_t)(B+1)*4);\n",
                two_d = 2 * d, ms = mscratch)
        },
        pfn = if prefill_t > 1 {
            // PT prompt tokens in one pass. The LM head is deliberately absent: prefill needs the
            // hidden state, not the logits, and the head is the single widest matmul in the program
            // (VOCAB=248320). Running it once per prompt token is pure waste.
            // ★ T IS A RUNTIME ARGUMENT, not the compile-time PT.
            //
            // It used to be baked, and `mtp_verify` calls this function: a speculative verify has 2
            // tokens, so with PT baked the whole program had to be emitted at PT=2 for MTP to run at
            // all — which made PREFILL take 16.04 s instead of 11.66 s, a 4.4 s regression that
            // swamped the +6.9% the speculation earned. Prefill wants a wide chunk; verify wants two
            // rows; they are not the same number and there is no reason they should share one.
            //
            // The buffers stay sized for the WIDEST (B*PT), so a narrow call simply uses fewer rows.
            // ── THE PER-LAYER ACCURACY PROBE ──────────────────────────────
            // A lane that is FASTER and produces DIFFERENT TOKENS has proved nothing: this campaign has
            // already shipped five fast-and-wrong results and three of them were faster than the correct
            // code. Token identity is not the gate -- the residual stream is. `x_f32` at each layer's
            // entry is norm(x + tmp), the fully accumulated state, so dumping it per layer localises a
            // divergence to the layer that caused it and separates a LAYOUT bug (one tile garbage, the
            // rest exact) from fp8 PRECISION (a smooth ~1e-3 that grows). First chunk only, first and
            // last M-tile rows only, so the probe costs ~10 MB instead of 21 GB.
            format!("static FILE* bq_tf=nullptr; static float* bq_th=nullptr; static int bq_tchunk=0;\n\
static void bq_trace(cudaStream_t s,const float* p,long off,long n){{\n\
\x20 if(!bq_tf||bq_tchunk!=1) return;\n\
\x20 cudaStreamSynchronize(s);\n\
\x20 cudaMemcpy(bq_th,p+off,(size_t)n*4,cudaMemcpyDeviceToHost);\n\
\x20 fwrite(bq_th,4,(size_t)n,bq_tf);\n\
}}\n\
static void bq_trace_rows(cudaStream_t s,const float* p,int rows,int w){{\n\
\x20 if(!bq_tf||bq_tchunk!=1) return;\n\
\x20 const int r = rows<4?rows:4;\n\
\x20 bq_trace(s,p,0,(long)r*w);\n\
\x20 bq_trace(s,p,(long)(rows-r)*w,(long)r*w);\n\
}}\n\
static void prefill_chunk(cudaStream_t s, int MID, int PTv){{\n\
                     \x20 const int BTv = B*PTv; ++bq_tchunk;\n\
                     \x20 bq_embed<<<dim3(Gs(E),BTv),256,0,s>>>(x,emb_q,emb_s,d_tok,E);\n\
                     {pstep}}}\n\n",
                    pstep = replace_word(&replace_word(&pstep, "BT", "BTv"), "PT", "PTv"))
        } else { String::new() },
        tracopen = {
            let dab = concat!("  { const char* _db=getenv(\"LOGOS_Q38B_DABLATE\");\n",
                    "    if(_db){ const char* g[5]={\"gdn\",\"conv\",\"attn\",\"norm\",\"ffn\"};\n",
                    "      for(int i=0;i<5;++i) if(!strcmp(_db,g[i])) bq_da[i]=1; }\n",
                    "    fprintf(stderr,\"DABLATE %s\\n\", _db?_db:\"none\"); }\n");
            let rest = if prefill_t > 1 {
            concat!("#ifdef BQ_ENABLE_DG\n",
                    "  { const char* _ds=getenv(\"LOGOS_Q38B_DGSITES\");\n",
                    "    if(_ds) for(int i=0;i<3&&_ds[i];++i) bq_dg_on[i] = (_ds[i]=='1');\n",
                    "    const char* _dh=getenv(\"LOGOS_Q38B_DGHL\"); if(_dh) bq_dg_hl = (_dh[0]!='0');\n",
                    "    fprintf(stderr,\"DGHL %d\\n\",bq_dg_hl);\n",
                    "    fprintf(stderr,\"DGSITES %d%d%d\\n\",bq_dg_on[0],bq_dg_on[1],bq_dg_on[2]); }\n",
                    "#endif\n",
                    "  { const char* _tp=getenv(\"LOGOS_Q38B_TRACE\"); if(_tp){ bq_tf=fopen(_tp,\"wb\"); bq_th=(float*)malloc((size_t)8*E*4); } }\n")
            } else { "" };
            format!("{dab}{rest}")
        },
        skp_alloc = if tc_part_max.get() > 0 {
            // Split-K partials: the widest (split x N) any projection asked for, times the batch.
            format!("  cudaMalloc(&skp,(size_t){}*BT*4);\n", tc_part_max.get())
        } else { String::new() },
        tc_tj = tc_tj, tc_wm = tc_wm, tc_kc = tc_kc, tc_occ_def = tc_occ_def,
        q4_def = if use_q4 { "#define BQ_ENABLE_Q4 1\n" } else { "" },
        tc_lanes = if hl { 2 } else { 1 }, tc_stages = tc_stages, prefill_t = prefill_t,
        mv_tj = mv_tj, mv_rb = mv_rb, mv_nw = mv_nw, rmsn_nt = rmsn_nt, norm_g = norm_g,
        norm_alloc = if norm_split {
            // NORM_G partial sums per sequence — kilobytes, allocated only on the fast lane.
            "  cudaMalloc(&normp,(size_t)NORM_G*B*4);\n".to_string()
        } else { String::new() },
        mmq_tj = mmq_tj,
        mmq_acc = mmq_acc,
        head = head_launch,
        pmax = projw.max(attnw), two_dff = 2 * dff, two_nv = 2 * nv,
        state_mb = cfg.state_bytes_per_seq(maxseq) as f64 / 1e6,
        prompt_list = prompt.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(","),
    )
}
