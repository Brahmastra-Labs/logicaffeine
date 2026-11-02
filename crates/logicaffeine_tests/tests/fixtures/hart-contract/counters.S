# The counters: user mode reads cycle, time and instret only where mcounteren allows; an instruction that writes
# minstret does not also count itself; mcountinhibit freezes the counter it names.

#include "riscv_test.h"
#include "test_macros.h"
#include "common.h"

RVTEST_RV64M
RVTEST_CODE_BEGIN
  INSTALL_HANDLER

  csrwi mcounteren, 0
  li TESTNUM, 2
  li s2, 0
  ENTER_USER(1f, 2f)
1: rdcycle a0
  ecall
2:
  EXPECT(CAUSE_ILLEGAL_INSTRUCTION)
  li TESTNUM, 3
  li s2, 0
  ENTER_USER(3f, 4f)
3: rdtime a0
  ecall
4:
  EXPECT(CAUSE_ILLEGAL_INSTRUCTION)
  li TESTNUM, 4
  li s2, 0
  ENTER_USER(5f, 6f)
5: rdinstret a0
  ecall
6:
  EXPECT(CAUSE_ILLEGAL_INSTRUCTION)

  li TESTNUM, 5
  csrwi mcounteren, 7
  li s2, 0
  ENTER_USER(7f, 8f)
7:
  rdcycle a0
  rdtime a1
  rdinstret a2
  ecall
8:
  EXPECT(CAUSE_USER_ECALL)

  li TESTNUM, 6
  .option push
  .option norvc
  csrw minstret, zero
  csrr t0, minstret
  nop
  csrr t1, minstret
  .option pop
  bnez t0, fail
  li t2, 2; bne t1, t2, fail

  li TESTNUM, 7
  csrsi mcountinhibit, 4
  csrr t0, minstret
  nop
  nop
  csrr t1, minstret
  bne t0, t1, fail
  csrci mcountinhibit, 4
  csrr t1, minstret
  beq t0, t1, fail

  CONTRACT_VERDICT
  CONTRACT_HANDLER

RVTEST_CODE_END

  .data
RVTEST_DATA_BEGIN
RVTEST_DATA_END
