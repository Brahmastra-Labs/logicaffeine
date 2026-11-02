# The hart traps a misaligned load, store or atomic and leaves the access undone: the cause, the address in mtval,
# no register written, no memory changed. The board's firmware emulates the access from exactly this.

#include "riscv_test.h"
#include "test_macros.h"
#include "common.h"

RVTEST_RV64M
RVTEST_CODE_BEGIN
  .option norvc
  INSTALL_HANDLER
  la s0, data

#define EXPECT_TRAP(cause, addr)        \
  li t2, 1; bne s2, t2, fail;           \
  li t2, cause; bne s4, t2, fail;       \
  bne s5, addr, fail;                   \
  li s2, 0

#define LOAD_TRAPS(n, insn, off)        \
  li TESTNUM, n;                        \
  li s2, 0;                             \
  addi s3, s0, off;                     \
  li t1, 0x5a5a;                        \
  insn t1, off(s0);                     \
  EXPECT_TRAP(CAUSE_MISALIGNED_LOAD, s3); \
  li t2, 0x5a5a; bne t1, t2, fail

#define STORE_TRAPS(n, insn, off)       \
  li TESTNUM, n;                        \
  li s2, 0;                             \
  addi s3, s0, off;                     \
  li t1, -1;                            \
  insn t1, off(s0);                     \
  EXPECT_TRAP(CAUSE_MISALIGNED_STORE, s3); \
  ld t2, 0(s0); ld t4, guard; bne t2, t4, fail; \
  ld t2, 8(s0); ld t4, guard+8; bne t2, t4, fail

  LOAD_TRAPS(2, lh, 1)
  LOAD_TRAPS(3, lhu, 3)
  LOAD_TRAPS(4, lw, 2)
  LOAD_TRAPS(5, lwu, 1)
  LOAD_TRAPS(6, ld, 4)
  LOAD_TRAPS(7, ld, 7)
  STORE_TRAPS(8, sh, 1)
  STORE_TRAPS(9, sw, 2)
  STORE_TRAPS(10, sd, 4)

  # An atomic is a store for its cause, and changes nothing.
  li TESTNUM, 11
  li s2, 0
  addi s3, s0, 2
  li t5, 7
  li t1, 0x5a5a
  amoadd.w t1, t5, (s3)
  EXPECT_TRAP(CAUSE_MISALIGNED_STORE, s3)
  li t2, 0x5a5a; bne t1, t2, fail
  ld t2, 0(s0); ld t4, guard; bne t2, t4, fail

  li TESTNUM, 12
  li s2, 0
  addi s3, s0, 4
  lr.d t1, (s3)
  EXPECT_TRAP(CAUSE_MISALIGNED_LOAD, s3)

  li TESTNUM, 13
  li s2, 0
  addi s3, s0, 4
  sc.d t1, t5, (s3)
  EXPECT_TRAP(CAUSE_MISALIGNED_STORE, s3)
  ld t2, 0(s0); ld t4, guard; bne t2, t4, fail

  CONTRACT_VERDICT

  CONTRACT_HANDLER

RVTEST_CODE_END

  .data
RVTEST_DATA_BEGIN
  .align 4
data:  .dword 0x0807060504030201, 0x100f0e0d0c0b0a09
guard: .dword 0x0807060504030201, 0x100f0e0d0c0b0a09
RVTEST_DATA_END
