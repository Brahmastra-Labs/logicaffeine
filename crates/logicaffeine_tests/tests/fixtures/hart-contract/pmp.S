# Physical memory protection as the board's firmware uses it to fence the compiler off from itself: user accesses
# need an entry that grants them, a machine access is bound only by a locked entry or by MPRV lending it user mode,
# an access that only partly matches an entry fails in every mode, and a locked entry can no longer be changed.

#include "riscv_test.h"
#include "test_macros.h"
#include "common.h"

#define ALL_RWX (PMP_NAPOT | PMP_R | PMP_W | PMP_X)

RVTEST_RV64M
RVTEST_CODE_BEGIN
  INSTALL_HANDLER
  la s0, page
  srli t0, s0, 2
  ori t0, t0, 0x1ff
  csrw pmpaddr0, t0
  li t0, (1 << (31 + (__riscv_xlen / 64) * (53 - 31))) - 1
  csrw pmpaddr1, t0
  li t0, (PMP_NAPOT | PMP_R) | (ALL_RWX << 8)
  csrw pmpcfg0, t0

  li TESTNUM, 2
  li s2, 0
  ENTER_USER(1f, 2f)
1: ld a0, 0(s0)
   sd a0, 8(s0)
   ecall
2:
  EXPECT(CAUSE_STORE_ACCESS)
  addi t1, s0, 8; bne s5, t1, fail

  li TESTNUM, 3
  li s2, 0
  li t0, MSTATUS_MPP; csrc mstatus, t0
  la s8, 3f
  csrw mepc, s0
  mret
3:
  EXPECT(CAUSE_FETCH_ACCESS)
  bne s5, s0, fail

  li TESTNUM, 4
  li s2, 0
  li t1, 77
  sd t1, 16(s0)
  bnez s2, fail
  ld t2, 16(s0); bne t1, t2, fail

  li TESTNUM, 5
  li s2, 0
  li t0, MSTATUS_MPP; csrc mstatus, t0
  li t0, MSTATUS_MPRV; csrs mstatus, t0
  sd t1, 24(s0)
  li t0, MSTATUS_MPRV; csrc mstatus, t0
  EXPECT(CAUSE_STORE_ACCESS)
  addi t1, s0, 24; bne s5, t1, fail

  li TESTNUM, 6
  srli t0, s0, 2
  csrw pmpaddr0, t0
  li t0, (PMP_NA4 | PMP_R) | (ALL_RWX << 8)
  csrw pmpcfg0, t0
  li s2, 0
  ld a0, 0(s0)
  EXPECT(CAUSE_LOAD_ACCESS)
  bne s5, s0, fail
  lw a0, 0(s0)
  li t2, 1; bne s2, t2, fail

  li TESTNUM, 7
  srli t0, s0, 2
  ori t0, t0, 0x1ff
  csrw pmpaddr0, t0
  li t0, (PMP_NAPOT | PMP_R | PMP_L) | (ALL_RWX << 8)
  csrw pmpcfg0, t0
  li s2, 0
  sd t1, 32(s0)
  EXPECT(CAUSE_STORE_ACCESS)
  csrr a1, pmpaddr0
  csrw pmpaddr0, zero
  csrr a2, pmpaddr0
  bne a1, a2, fail
  li t0, ALL_RWX << 8
  csrw pmpcfg0, t0
  csrr a1, pmpcfg0
  andi a1, a1, 0xff
  li t2, PMP_NAPOT | PMP_R | PMP_L
  bne a1, t2, fail

  CONTRACT_VERDICT
  CONTRACT_HANDLER

RVTEST_CODE_END

  .data
RVTEST_DATA_BEGIN
  .align 12
page: .fill 512, 8, 0
RVTEST_DATA_END
