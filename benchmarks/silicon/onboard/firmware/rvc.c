// The RV64C decompressor, in C: the same table as `hart::rvc::expand` (hart_shim_lock holds the two equal over all
// 65,536 encodings). A reserved encoding expands to 0, which no instruction is.

#include "shim.h"

static uint32_t bit(uint16_t c, int i) { return (c >> i) & 1; }
static uint32_t bits(uint16_t c, int hi, int lo) { return ((uint32_t)c >> lo) & ((1u << (hi - lo + 1)) - 1); }
static int32_t sext(uint32_t v, int width) { return (int32_t)(v << (32 - width)) >> (32 - width); }

static uint32_t r_type(uint32_t f7, uint32_t rs2, uint32_t rs1, uint32_t f3, uint32_t rd, uint32_t op) {
  return (f7 << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op;
}
static uint32_t i_type(int32_t imm, uint32_t rs1, uint32_t f3, uint32_t rd, uint32_t op) {
  return (((uint32_t)imm & 0xfff) << 20) | (rs1 << 15) | (f3 << 12) | (rd << 7) | op;
}
static uint32_t s_type(int32_t imm, uint32_t rs2, uint32_t rs1, uint32_t f3, uint32_t op) {
  uint32_t u = (uint32_t)imm;
  return (((u >> 5) & 0x7f) << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) | ((u & 0x1f) << 7) | op;
}
static uint32_t b_type(int32_t imm, uint32_t rs2, uint32_t rs1, uint32_t f3, uint32_t op) {
  uint32_t u = (uint32_t)imm;
  return (((u >> 12) & 1) << 31) | (((u >> 5) & 0x3f) << 25) | (rs2 << 20) | (rs1 << 15) | (f3 << 12) |
         (((u >> 1) & 0xf) << 8) | (((u >> 11) & 1) << 7) | op;
}
static uint32_t j_type(int32_t imm, uint32_t rd, uint32_t op) {
  uint32_t u = (uint32_t)imm;
  return (((u >> 20) & 1) << 31) | (((u >> 1) & 0x3ff) << 21) | (((u >> 11) & 1) << 20) | (((u >> 12) & 0xff) << 12) |
         (rd << 7) | op;
}

uint32_t rvc_expand(uint16_t c) {
  uint32_t f3 = bits(c, 15, 13), rd = bits(c, 11, 7), rs2 = bits(c, 6, 2), rdp = 8 + bits(c, 4, 2), rs1p = 8 + bits(c, 9, 7);
  int32_t imm6 = sext((bit(c, 12) << 5) | bits(c, 6, 2), 6);
  switch (c & 3) {
    case 0:
      switch (f3) {
        case 0: {
          uint32_t u = (bits(c, 12, 11) << 4) | (bits(c, 10, 7) << 6) | (bit(c, 6) << 2) | (bit(c, 5) << 3);
          return u ? i_type((int32_t)u, 2, 0, rdp, 0x13) : 0;
        }
        case 1: return i_type((int32_t)((bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6)), rs1p, 3, rdp, 0x07);
        case 2: return i_type((int32_t)((bits(c, 12, 10) << 3) | (bit(c, 6) << 2) | (bit(c, 5) << 6)), rs1p, 2, rdp, 0x03);
        case 3: return i_type((int32_t)((bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6)), rs1p, 3, rdp, 0x03);
        case 4: return 0;
        case 5: return s_type((int32_t)((bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6)), rdp, rs1p, 3, 0x27);
        case 6: return s_type((int32_t)((bits(c, 12, 10) << 3) | (bit(c, 6) << 2) | (bit(c, 5) << 6)), rdp, rs1p, 2, 0x23);
        default: return s_type((int32_t)((bits(c, 12, 10) << 3) | (bits(c, 6, 5) << 6)), rdp, rs1p, 3, 0x23);
      }
    case 1:
      switch (f3) {
        case 0: return i_type(imm6, rd, 0, rd, 0x13);
        case 1: return rd ? i_type(imm6, rd, 0, rd, 0x1b) : 0;
        case 2: return i_type(imm6, 0, 0, rd, 0x13);
        case 3:
          if (rd == 2) {
            uint32_t n = (bit(c, 12) << 9) | (bit(c, 6) << 4) | (bit(c, 5) << 6) | (bits(c, 4, 3) << 7) | (bit(c, 2) << 5);
            return n ? i_type(sext(n, 10), 2, 0, 2, 0x13) : 0;
          } else {
            uint32_t n = (bit(c, 12) << 5) | bits(c, 6, 2);
            return n ? ((((uint32_t)sext(n, 6)) & 0xfffff) << 12) | (rd << 7) | 0x37 : 0;
          }
        case 4: {
          uint32_t sh = (bit(c, 12) << 5) | bits(c, 6, 2);
          switch (bits(c, 11, 10)) {
            case 0: return i_type((int32_t)sh, rs1p, 5, rs1p, 0x13);
            case 1: return i_type((int32_t)(0x400 | sh), rs1p, 5, rs1p, 0x13);
            case 2: return i_type(imm6, rs1p, 7, rs1p, 0x13);
            default: {
              uint32_t rs2p = 8 + bits(c, 4, 2), sel = (bit(c, 12) << 2) | bits(c, 6, 5);
              switch (sel) {
                case 0: return r_type(0x20, rs2p, rs1p, 0, rs1p, 0x33);
                case 1: return r_type(0, rs2p, rs1p, 4, rs1p, 0x33);
                case 2: return r_type(0, rs2p, rs1p, 6, rs1p, 0x33);
                case 3: return r_type(0, rs2p, rs1p, 7, rs1p, 0x33);
                case 4: return r_type(0x20, rs2p, rs1p, 0, rs1p, 0x3b);
                case 5: return r_type(0, rs2p, rs1p, 0, rs1p, 0x3b);
                default: return 0;
              }
            }
          }
        }
        case 5: {
          uint32_t i = (bit(c, 12) << 11) | (bit(c, 11) << 4) | (bits(c, 10, 9) << 8) | (bit(c, 8) << 10) | (bit(c, 7) << 6) |
                       (bit(c, 6) << 7) | (bits(c, 5, 3) << 1) | (bit(c, 2) << 5);
          return j_type(sext(i, 12), 0, 0x6f);
        }
        default: {
          uint32_t i = (bit(c, 12) << 8) | (bits(c, 11, 10) << 3) | (bits(c, 6, 5) << 6) | (bits(c, 4, 3) << 1) | (bit(c, 2) << 5);
          return b_type(sext(i, 9), 0, rs1p, f3 == 6 ? 0 : 1, 0x63);
        }
      }
    case 2:
      switch (f3) {
        case 0: return i_type((int32_t)((bit(c, 12) << 5) | bits(c, 6, 2)), rd, 1, rd, 0x13);
        case 1: return i_type((int32_t)((bit(c, 12) << 5) | (bits(c, 6, 5) << 3) | (bits(c, 4, 2) << 6)), 2, 3, rd, 0x07);
        case 2: return rd ? i_type((int32_t)((bit(c, 12) << 5) | (bits(c, 6, 4) << 2) | (bits(c, 3, 2) << 6)), 2, 2, rd, 0x03) : 0;
        case 3: return rd ? i_type((int32_t)((bit(c, 12) << 5) | (bits(c, 6, 5) << 3) | (bits(c, 4, 2) << 6)), 2, 3, rd, 0x03) : 0;
        case 4:
          if (!bit(c, 12)) {
            if (rs2 == 0) return rd ? i_type(0, rd, 0, 0, 0x67) : 0;
            return r_type(0, rs2, 0, 0, rd, 0x33);
          }
          if (rs2 == 0 && rd == 0) return i_type(1, 0, 0, 0, 0x73);
          if (rs2 == 0) return i_type(0, rd, 0, 1, 0x67);
          return r_type(0, rs2, rd, 0, rd, 0x33);
        case 5: return s_type((int32_t)((bits(c, 12, 10) << 3) | (bits(c, 9, 7) << 6)), rs2, 2, 3, 0x27);
        case 6: return s_type((int32_t)((bits(c, 12, 9) << 2) | (bits(c, 8, 7) << 6)), rs2, 2, 2, 0x23);
        default: return s_type((int32_t)((bits(c, 12, 10) << 3) | (bits(c, 9, 7) << 6)), rs2, 2, 3, 0x23);
      }
    default:
      return 0;
  }
}
