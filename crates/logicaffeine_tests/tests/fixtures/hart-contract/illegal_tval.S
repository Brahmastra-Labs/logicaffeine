# Every floating-point instruction, and every access to the floating-point CSRs, is an illegal instruction whose
# mtval is its exact encoding — 32 bits, or the 16 bits of a compressed one — because the board's firmware decodes
# and emulates it from mtval alone.

#include "riscv_test.h"
#include "test_macros.h"
#include "common.h"

RVTEST_RV64M
RVTEST_CODE_BEGIN
  INSTALL_HANDLER

#define ILLEGAL32(n, label)                       \
  li TESTNUM, n;                                  \
  li s2, 0;                                       \
  la t0, label; lhu s3, 0(t0); lhu t1, 2(t0);     \
  slli t1, t1, 16; or s3, s3, t1;                 \
  j 1f;                                           \
1:

#define EXPECT_ILLEGAL                            \
  li t2, 1; bne s2, t2, fail;                     \
  li t2, CAUSE_ILLEGAL_INSTRUCTION; bne s4, t2, fail; \
  bne s5, s3, fail

  .option push
  .option arch, +d
  .option norvc
  ILLEGAL32(2, i_fadd)
i_fadd: fadd.d f1, f2, f3
  EXPECT_ILLEGAL
  ILLEGAL32(3, i_fld)
i_fld:  fld f4, 16(sp)
  EXPECT_ILLEGAL
  ILLEGAL32(4, i_fsd)
i_fsd:  fsd f5, 8(sp)
  EXPECT_ILLEGAL
  ILLEGAL32(5, i_fmv)
i_fmv:  fmv.x.d a0, f6
  EXPECT_ILLEGAL
  ILLEGAL32(6, i_fcsr)
i_fcsr: csrr a0, fcsr
  EXPECT_ILLEGAL
  ILLEGAL32(7, i_frm)
i_frm:  csrwi frm, 3
  EXPECT_ILLEGAL
  ILLEGAL32(8, i_fflags)
i_fflags: csrr a0, fflags
  EXPECT_ILLEGAL
  ILLEGAL32(9, i_fmadd)
i_fmadd: fmadd.d f1, f2, f3, f4
  EXPECT_ILLEGAL

  # Compressed floating-point loads and stores: mtval holds the 16-bit encoding, zero-extended.
#define ILLEGAL16(n, label)                       \
  li TESTNUM, n;                                  \
  li s2, 0;                                       \
  la t0, label; lhu s3, 0(t0);                    \
  j 1f;                                           \
1:
  .option rvc
  ILLEGAL16(10, c_fldsp)
c_fldsp: c.fldsp f8, 24(sp)
  .option norvc
  EXPECT_ILLEGAL
  .option rvc
  ILLEGAL16(11, c_fsdsp)
c_fsdsp: c.fsdsp f9, 32(sp)
  .option norvc
  EXPECT_ILLEGAL
  .option rvc
  ILLEGAL16(12, c_fld)
c_fld:   c.fld f10, 8(s0)
  .option norvc
  EXPECT_ILLEGAL
  .option pop

  # A reserved compressed encoding reports its 16 bits too.
  li TESTNUM, 13
  li s2, 0
  li s3, 0x6101
  .option push
  .option rvc
  .2byte 0x6101
  .option pop
  EXPECT_ILLEGAL

  CONTRACT_VERDICT

  CONTRACT_HANDLER

RVTEST_CODE_END

  .data
RVTEST_DATA_BEGIN
RVTEST_DATA_END
