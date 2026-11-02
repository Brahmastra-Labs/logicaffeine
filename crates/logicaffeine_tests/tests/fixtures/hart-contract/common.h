// The contract tests' trap handler and verdict tail. The handler counts traps in s2 and records mcause in s4,
// mtval in s5, mepc in s6 and mstatus in s7. With a resume address in s8 it returns there in machine mode (s8 is
// then cleared); otherwise it returns past the trapping instruction, whatever its length.

#define CONTRACT_HANDLER                          \
  .align 2;                                       \
handler:                                          \
  addi s2, s2, 1;                                 \
  csrr s4, mcause;                                \
  csrr s5, mtval;                                 \
  csrr s6, mepc;                                  \
  csrr s7, mstatus;                               \
  beqz s8, 2f;                                    \
  csrw mepc, s8;                                  \
  li s8, 0;                                       \
  li t3, MSTATUS_MPP;                             \
  csrs mstatus, t3;                               \
  mret;                                           \
2:                                                \
  lhu t4, 0(s6);                                  \
  andi t4, t4, 3;                                 \
  li t6, 3;                                       \
  addi t3, s6, 2;                                 \
  bne t4, t6, 3f;                                 \
  addi t3, t3, 2;                                 \
3:                                                \
  csrw mepc, t3;                                  \
  mret

#define CONTRACT_VERDICT                          \
  j pass;                                         \
fail:                                             \
  la t0, trap_vector;                             \
  csrw mtvec, t0;                                 \
  RVTEST_FAIL;                                    \
pass:                                             \
  la t0, trap_vector;                             \
  csrw mtvec, t0;                                 \
  RVTEST_PASS

#define INSTALL_HANDLER                           \
  la t0, handler;                                 \
  csrw mtvec, t0;                                 \
  li s8, 0

// Drop to user mode at `label`; a trap there returns to `resume` in machine mode. A user-mode probe ends in `ecall`,
// so one that fails to trap reports cause 8 instead of running on into machine-mode test code.
#define ENTER_USER(label, resume)                 \
  li t0, MSTATUS_MPP;                             \
  csrc mstatus, t0;                               \
  la s8, resume;                                  \
  la t0, label;                                   \
  csrw mepc, t0;                                  \
  mret

#define EXPECT(cause)                             \
  li t2, 1; bne s2, t2, fail;                     \
  li t2, cause; bne s4, t2, fail
