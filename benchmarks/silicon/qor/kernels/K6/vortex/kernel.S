// K6 simt_axpb_sum on Vortex: WARPS warps of four threads, a thread per lane, over the BATCH operations of
// one launch; warp w takes operations w, w + WARPS, w + 2 WARPS, ... Operation j's operands are at
// 0x10000 + 56 j (a, b, mask, x0..x3, eight bytes each) and its results go to 0x20000 + 40 j (y0..y3, then
// the sum). Each thread computes its lane's a * x + b, zero where its mask bit is clear, stores it, and sums
// the four lanes with two butterfly shuffles across its warp; every thread holds the sum and stores the
// same value. RV64IM with Vortex's custom-0 shuffle, warp-sync and thread-mask instructions.
  .option norvc
  .section .text
  .global _start
_start:
  csrr  t0, 0xCC0              // this thread's lane
  csrr  t4, 0xCC1              // this thread's warp
  li    t5, 56
  mul   t5, t5, t4
  li    s0, 0x10000
  add   s0, s0, t5
  li    t5, 40
  mul   t5, t5, t4
  li    s1, 0x20000
  add   s1, s1, t5
  li    s2, BATCH / WARPS
  slli  s3, t0, 3
  li    s5, 0x3C0C1            // butterfly with lane ^ 1 among the warp's four lanes
  li    s6, 0x3C0C2            // butterfly with lane ^ 2
1:
  ld    a1, 0(s0)              // a
  ld    a2, 8(s0)              // b
  ld    a3, 16(s0)             // mask
  add   t1, s0, s3
  ld    a4, 24(t1)             // x of this lane
  mul   a5, a1, a4
  add   a5, a5, a2
  srl   t2, a3, t0
  andi  t2, t2, 1
  neg   t2, t2
  and   a5, a5, t2
  add   t3, s1, s3
  sd    a5, 0(t3)              // y of this lane
  .insn r 0x0B, 6, 1, a6, a5, s5
  add   a6, a6, a5
  .insn r 0x0B, 6, 1, a7, a6, s6
  add   a7, a7, a6
  sd    a7, 32(s1)             // the sum
  addi  s0, s0, 56 * WARPS
  addi  s1, s1, 40 * WARPS
  addi  s2, s2, -1
  bnez  s2, 1b
  .insn r 0x0B, 7, 0, x0, x0, x0   // wsync: drain the warp's outstanding instructions
  .insn r 0x0B, 0, 0, x0, x0, x0   // tmc x0: the warp ends
