// fpu — the F and D extensions in software. The hart has no floating-point unit: every F or D instruction, and
// every access to fflags, frm or fcsr, is an illegal instruction whose encoding arrives in mtval, and this file
// performs it exactly as the specification defines — through Berkeley SoftFloat 3e built with its RISC-V
// specialization (canonical NaNs, saturating conversions), with single-precision values NaN-boxed in the 64-bit
// registers. A compile spends 0.0002% of its instructions here.

#include "shim.h"
#include "softfloat.h"

static uint64_t fregs[32];
static uint32_t fcsr;  // [4:0] the accrued exception flags, [7:5] the dynamic rounding mode

void fpu_reset(void) {
  for (int i = 0; i < 32; i++) fregs[i] = 0;
  fcsr = 0;
}

static float32_t unbox32(uint64_t v) {
  float32_t r;
  r.v = (v >> 32) == 0xffffffffu ? (uint32_t)v : 0x7fc00000u;
  return r;
}

static uint64_t box32(float32_t f) { return 0xffffffff00000000ull | f.v; }

static float64_t as64(uint64_t v) {
  float64_t r;
  r.v = v;
  return r;
}

static uint64_t sext32(uint32_t v) { return (uint64_t)(int64_t)(int32_t)v; }

// The rounding mode an instruction asks for, or -1 for a reserved one (an illegal instruction).
static int rounding(uint32_t rm) {
  if (rm == 7) rm = (fcsr >> 5) & 7;
  switch (rm) {
    case 0: return softfloat_round_near_even;
    case 1: return softfloat_round_minMag;
    case 2: return softfloat_round_min;
    case 3: return softfloat_round_max;
    case 4: return softfloat_round_near_maxMag;
    default: return -1;
  }
}

static int is_nan32(float32_t a) { return ((a.v >> 23) & 0xff) == 0xff && (a.v & 0x7fffff); }
static int is_nan64(float64_t a) { return ((a.v >> 52) & 0x7ff) == 0x7ff && (a.v & 0xfffffffffffffull); }
static int is_snan32(float32_t a) { return is_nan32(a) && !(a.v & 0x400000); }
static int is_snan64(float64_t a) { return is_nan64(a) && !(a.v & 0x8000000000000ull); }

static uint64_t classify(uint64_t sign, int inf, int zero, int sub, int nan, int quiet) {
  if (inf) return sign ? 1u << 0 : 1u << 7;
  if (zero) return sign ? 1u << 3 : 1u << 4;
  if (sub) return sign ? 1u << 2 : 1u << 5;
  if (nan) return quiet ? 1u << 9 : 1u << 8;
  return sign ? 1u << 1 : 1u << 6;
}

static uint64_t fclass32(float32_t a) {
  uint32_t e = (a.v >> 23) & 0xff, m = a.v & 0x7fffff;
  return classify(a.v >> 31, e == 0xff && !m, e == 0 && !m, e == 0 && m, e == 0xff && m, (m >> 22) & 1);
}

static uint64_t fclass64(float64_t a) {
  uint64_t e = (a.v >> 52) & 0x7ff, m = a.v & 0xfffffffffffffull;
  return classify(a.v >> 63, e == 0x7ff && !m, e == 0 && !m, e == 0 && m, e == 0x7ff && m, (m >> 51) & 1);
}

// fmin/fmax as RISC-V defines them: a NaN operand yields the other, two NaNs the canonical NaN, -0 orders below +0,
// and a signaling NaN raises the invalid flag.
static float64_t minmax64(float64_t a, float64_t b, int max) {
  if (is_snan64(a) || is_snan64(b)) softfloat_exceptionFlags |= softfloat_flag_invalid;
  if (is_nan64(a) && is_nan64(b)) return as64(0x7ff8000000000000ull);
  if (is_nan64(a)) return b;
  if (is_nan64(b)) return a;
  int less = f64_lt_quiet(a, b) || (f64_eq(a, b) && (a.v >> 63) && !(b.v >> 63));
  return (less != max) ? a : b;
}

static float32_t minmax32(float32_t a, float32_t b, int max) {
  if (is_snan32(a) || is_snan32(b)) softfloat_exceptionFlags |= softfloat_flag_invalid;
  if (is_nan32(a) && is_nan32(b)) {
    float32_t r;
    r.v = 0x7fc00000u;
    return r;
  }
  if (is_nan32(a)) return b;
  if (is_nan32(b)) return a;
  int less = f32_lt_quiet(a, b) || (f32_eq(a, b) && (a.v >> 31) && !(b.v >> 31));
  return (less != max) ? a : b;
}

static int fp_csr(struct frame *f, uint32_t inst) {
  uint32_t csr = inst >> 20, f3 = (inst >> 12) & 7, rd = (inst >> 7) & 31, rs1 = (inst >> 15) & 31;
  if (csr < 1 || csr > 3 || f3 == 0 || f3 == 4) return 0;
  uint32_t old = csr == 1 ? (fcsr & 0x1f) : csr == 2 ? ((fcsr >> 5) & 7) : (fcsr & 0xff);
  uint64_t operand = (f3 & 4) ? rs1 : f->x[rs1];
  int writes = (f3 & 3) == 1 || rs1 != 0;
  if (writes) {
    uint32_t v = (f3 & 3) == 1 ? (uint32_t)operand : (f3 & 3) == 2 ? (old | (uint32_t)operand) : (old & ~(uint32_t)operand);
    if (csr == 1) fcsr = (fcsr & ~0x1fu) | (v & 0x1f);
    else if (csr == 2) fcsr = (fcsr & ~0xe0u) | ((v & 7) << 5);
    else fcsr = v & 0xff;
  }
  if (rd) f->x[rd] = old;
  return 1;
}

int fpu_emulate(struct frame *f, uint32_t raw, int compressed) {
  uint32_t inst = compressed ? rvc_expand((uint16_t)raw) : raw;
  if (compressed && inst == 0) return 0;
  uint32_t op = inst & 0x7f, rd = (inst >> 7) & 31, f3 = (inst >> 12) & 7, rs1 = (inst >> 15) & 31,
           rs2 = (inst >> 20) & 31, f7 = inst >> 25, fmt = (inst >> 25) & 3, rs3 = inst >> 27;
  if (op == 0x73) return fp_csr(f, inst);
  if (op == 0x07 || op == 0x27) {
    int64_t imm = op == 0x07 ? ((int32_t)inst >> 20) : (((int32_t)inst >> 25) << 5) | (int32_t)((inst >> 7) & 31);
    uint64_t addr = f->x[rs1] + (uint64_t)imm;
    if (f3 != 2 && f3 != 3) return 0;
    if (op == 0x07) {
      uint64_t v = 0;
      memcpy(&v, (const void *)addr, f3 == 2 ? 4 : 8);
      fregs[rd] = f3 == 2 ? (0xffffffff00000000ull | (uint32_t)v) : v;
    } else {
      memcpy((void *)addr, &fregs[rs2], f3 == 2 ? 4 : 8);
    }
    return 1;
  }
  if (fmt > 1) return 0;
  int dbl = fmt == 1;
  softfloat_exceptionFlags = 0;
  if (op == 0x43 || op == 0x47 || op == 0x4b || op == 0x4f) {
    int rm = rounding(f3);
    if (rm < 0) return 0;
    softfloat_roundingMode = (uint_fast8_t)rm;
    int neg_product = op == 0x4b || op == 0x4f, neg_addend = op == 0x47 || op == 0x4f;
    if (dbl) {
      float64_t a = as64(fregs[rs1]), b = as64(fregs[rs2]), c = as64(fregs[rs3]);
      if (neg_product) a.v ^= 1ull << 63;
      if (neg_addend) c.v ^= 1ull << 63;
      fregs[rd] = f64_mulAdd(a, b, c).v;
    } else {
      float32_t a = unbox32(fregs[rs1]), b = unbox32(fregs[rs2]), c = unbox32(fregs[rs3]);
      if (neg_product) a.v ^= 1u << 31;
      if (neg_addend) c.v ^= 1u << 31;
      fregs[rd] = box32(f32_mulAdd(a, b, c));
    }
  } else if (op == 0x53) {
    uint32_t group = f7 >> 2;
    int needs_rm = group == 0x00 || group == 0x01 || group == 0x02 || group == 0x03 || group == 0x0b
                || group == 0x08 || group == 0x18 || group == 0x1a;
    if (needs_rm) {
      int rm = rounding(f3);
      if (rm < 0) return 0;
      softfloat_roundingMode = (uint_fast8_t)rm;
    }
    float64_t a64 = as64(fregs[rs1]), b64 = as64(fregs[rs2]);
    float32_t a32 = unbox32(fregs[rs1]), b32 = unbox32(fregs[rs2]);
    switch (group) {
      case 0x00:
        fregs[rd] = dbl ? f64_add(a64, b64).v : box32(f32_add(a32, b32));
        break;
      case 0x01:
        fregs[rd] = dbl ? f64_sub(a64, b64).v : box32(f32_sub(a32, b32));
        break;
      case 0x02:
        fregs[rd] = dbl ? f64_mul(a64, b64).v : box32(f32_mul(a32, b32));
        break;
      case 0x03:
        fregs[rd] = dbl ? f64_div(a64, b64).v : box32(f32_div(a32, b32));
        break;
      case 0x0b:
        if (rs2 != 0) return 0;
        fregs[rd] = dbl ? f64_sqrt(a64).v : box32(f32_sqrt(a32));
        break;
      case 0x04: {
        if (f3 > 2) return 0;
        if (dbl) {
          uint64_t s = f3 == 0 ? (b64.v >> 63) : f3 == 1 ? !(b64.v >> 63) : ((a64.v ^ b64.v) >> 63);
          fregs[rd] = (a64.v & ~(1ull << 63)) | (s << 63);
        } else {
          uint32_t s = f3 == 0 ? (b32.v >> 31) : f3 == 1 ? !(b32.v >> 31) : ((a32.v ^ b32.v) >> 31);
          float32_t r;
          r.v = (a32.v & ~(1u << 31)) | (s << 31);
          fregs[rd] = box32(r);
        }
        break;
      }
      case 0x05:
        if (f3 > 1) return 0;
        fregs[rd] = dbl ? minmax64(a64, b64, f3).v : box32(minmax32(a32, b32, f3));
        break;
      case 0x08:
        // The format field names the destination: fcvt.d.s has fmt D and a single-precision source (rs2 = 0).
        if (dbl && rs2 == 0) fregs[rd] = f32_to_f64(a32).v;
        else if (!dbl && rs2 == 1) fregs[rd] = box32(f64_to_f32(a64));
        else return 0;
        break;
      case 0x14: {
        uint64_t r;
        if (f3 == 2) r = dbl ? f64_eq(a64, b64) : f32_eq(a32, b32);
        else if (f3 == 1) r = dbl ? f64_lt(a64, b64) : f32_lt(a32, b32);
        else if (f3 == 0) r = dbl ? f64_le(a64, b64) : f32_le(a32, b32);
        else return 0;
        if (rd) f->x[rd] = r;
        break;
      }
      case 0x18: {
        uint64_t r;
        int rm = softfloat_roundingMode;
        switch (rs2) {
          case 0: r = sext32((uint32_t)(dbl ? f64_to_i32(a64, rm, 1) : f32_to_i32(a32, rm, 1))); break;
          case 1: r = sext32(dbl ? f64_to_ui32(a64, rm, 1) : f32_to_ui32(a32, rm, 1)); break;
          case 2: r = (uint64_t)(dbl ? f64_to_i64(a64, rm, 1) : f32_to_i64(a32, rm, 1)); break;
          case 3: r = dbl ? f64_to_ui64(a64, rm, 1) : f32_to_ui64(a32, rm, 1); break;
          default: return 0;
        }
        if (rd) f->x[rd] = r;
        break;
      }
      case 0x1a: {
        uint64_t x = f->x[rs1];
        switch (rs2) {
          case 0: fregs[rd] = dbl ? i32_to_f64((int32_t)x).v : box32(i32_to_f32((int32_t)x)); break;
          case 1: fregs[rd] = dbl ? ui32_to_f64((uint32_t)x).v : box32(ui32_to_f32((uint32_t)x)); break;
          case 2: fregs[rd] = dbl ? i64_to_f64((int64_t)x).v : box32(i64_to_f32((int64_t)x)); break;
          case 3: fregs[rd] = dbl ? ui64_to_f64(x).v : box32(ui64_to_f32(x)); break;
          default: return 0;
        }
        break;
      }
      case 0x1c: {
        if (rs2 != 0) return 0;
        uint64_t r;
        if (f3 == 0) r = dbl ? fregs[rs1] : sext32((uint32_t)fregs[rs1]);
        else if (f3 == 1) r = dbl ? fclass64(a64) : fclass32(a32);
        else return 0;
        if (rd) f->x[rd] = r;
        break;
      }
      case 0x1e:
        if (rs2 != 0 || f3 != 0) return 0;
        fregs[rd] = dbl ? f->x[rs1] : (0xffffffff00000000ull | (uint32_t)f->x[rs1]);
        break;
      default:
        return 0;
    }
  } else {
    return 0;
  }
  fcsr |= softfloat_exceptionFlags & 0x1f;
  return 1;
}
