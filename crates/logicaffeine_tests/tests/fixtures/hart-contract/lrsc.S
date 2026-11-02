# Reservations: an SC succeeds only on the address the LR reserved with nothing between them that releases it —
# a trap, an mret, or another SC.

#include "riscv_test.h"
#include "test_macros.h"
#include "common.h"

RVTEST_RV64M
RVTEST_CODE_BEGIN
  INSTALL_HANDLER
  la s0, word
  li t5, 0x1234

  li TESTNUM, 2
  sd zero, 0(s0)
  lr.d t1, (s0)
  sc.d t2, t5, (s0)
  bnez t2, fail
  ld t1, 0(s0); bne t1, t5, fail

  li TESTNUM, 3
  sd zero, 0(s0)
  lr.d t1, (s0)
  li s2, 0
  ecall
  sc.d t2, t5, (s0)
  li t1, 1; bne t2, t1, fail
  ld t1, 0(s0); bnez t1, fail

  li TESTNUM, 4
  sc.d t2, t5, (s0)
  li t1, 1; bne t2, t1, fail

  li TESTNUM, 5
  lr.d t1, (s0)
  addi t0, s0, 8
  sc.d t2, t5, (t0)
  li t1, 1; bne t2, t1, fail
  ld t1, 8(s0); bnez t1, fail

  li TESTNUM, 6
  lr.d t1, (s0)
  sc.d t2, t5, (s0)
  bnez t2, fail
  sc.d t2, zero, (s0)
  li t1, 1; bne t2, t1, fail

  li TESTNUM, 7
  sd zero, 0(s0)
  la t0, sc_in_trap
  csrw mtvec, t0
  li s10, 0
  lr.d t1, (s0)
  ecall
  INSTALL_HANDLER
  li t1, 1; bne s10, t1, fail
  ld t1, 0(s0); bnez t1, fail

  CONTRACT_VERDICT
  CONTRACT_HANDLER

  .align 2
sc_in_trap:
  sc.d s10, t5, (s0)
  csrr t3, mepc
  addi t3, t3, 4
  csrw mepc, t3
  mret

RVTEST_CODE_END

  .data
RVTEST_DATA_BEGIN
  .align 4
word: .dword 0, 0
RVTEST_DATA_END
