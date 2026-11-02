# Privilege transfer: mret restores MIE from MPIE, sets MPIE, leaves MPP at user mode and clears MPRV when it
# returns below machine mode; ecall reports the mode it came from with mepc at the ecall; user mode can neither
# return with mret nor read a machine CSR; WFI in user mode is illegal exactly when mstatus.TW is set.

#include "riscv_test.h"
#include "test_macros.h"
#include "common.h"

RVTEST_RV64M
RVTEST_CODE_BEGIN
  INSTALL_HANDLER

  li TESTNUM, 2
  li t0, MSTATUS_MPP; csrs mstatus, t0
  li t0, MSTATUS_MPIE; csrc mstatus, t0
  csrsi mstatus, MSTATUS_MIE
  la t0, 1f; csrw mepc, t0
  mret
1:
  csrr t1, mstatus
  li t2, MSTATUS_MIE; and t5, t1, t2; bnez t5, fail
  li t2, MSTATUS_MPIE; and t5, t1, t2; beqz t5, fail
  li t2, MSTATUS_MPP; and t5, t1, t2; bnez t5, fail

  li TESTNUM, 3
  li s2, 0
  la s3, 2f
2: ecall
  EXPECT(CAUSE_MACHINE_ECALL)
  bne s6, s3, fail

  li TESTNUM, 4
  li s2, 0
  ENTER_USER(3f, 4f)
3: ecall
4:
  EXPECT(CAUSE_USER_ECALL)
  la t1, 3b; bne s6, t1, fail
  li t2, MSTATUS_MPP; and t5, s7, t2; bnez t5, fail

  li TESTNUM, 5
  li s2, 0
  li t0, MSTATUS_MPRV; csrs mstatus, t0
  ENTER_USER(5f, 6f)
5: ecall
6:
  EXPECT(CAUSE_USER_ECALL)
  li t2, MSTATUS_MPRV; and t5, s7, t2; bnez t5, fail

  li TESTNUM, 6
  li t0, MSTATUS_MPRV | MSTATUS_MPP; csrs mstatus, t0
  la t0, 7f; csrw mepc, t0
  mret
7:
  csrr t1, mstatus
  li t2, MSTATUS_MPRV; and t5, t1, t2; beqz t5, fail
  csrc mstatus, t2

  li TESTNUM, 7
  li s2, 0
  ENTER_USER(8f, 9f)
8: mret
  ecall
9:
  EXPECT(CAUSE_ILLEGAL_INSTRUCTION)
  la t1, 8b; bne s6, t1, fail

  li TESTNUM, 8
  li s2, 0
  ENTER_USER(10f, 11f)
10: csrr a0, mstatus
  ecall
11:
  EXPECT(CAUSE_ILLEGAL_INSTRUCTION)

  li TESTNUM, 9
  li s2, 0
  li t0, MSTATUS_TW; csrs mstatus, t0
  ENTER_USER(12f, 13f)
12: wfi
  ecall
13:
  EXPECT(CAUSE_ILLEGAL_INSTRUCTION)

  li TESTNUM, 10
  li s2, 0
  li t0, MSTATUS_TW; csrc mstatus, t0
  ENTER_USER(14f, 15f)
14: wfi
  ecall
15:
  EXPECT(CAUSE_USER_ECALL)
  la t1, 14b; addi t1, t1, 4; bne s6, t1, fail

  CONTRACT_VERDICT
  CONTRACT_HANDLER

RVTEST_CODE_END

  .data
RVTEST_DATA_BEGIN
RVTEST_DATA_END
