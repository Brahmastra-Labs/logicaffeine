/* K5 ntt_bfly — one ML-KEM Cooley-Tukey butterfly with Montgomery fqmul (q = 3329, q^-1 = -3327 mod 2^16),
   as logicaffeine_system::ntt computes it. Both 16-bit results return packed: hi in bits 31:16, lo in 15:0. */
#include <stdint.h>

static int16_t montgomery_reduce(int32_t a) {
  int16_t t = (int16_t)((int16_t)a * (int16_t)-3327);
  return (int16_t)((a - (int32_t)t * 3329) >> 16);
}

uint32_t ntt_bfly(int16_t a, int16_t b, int16_t zeta) {
  int16_t t = montgomery_reduce((int32_t)zeta * b);
  uint16_t hi = (uint16_t)(a + t);
  uint16_t lo = (uint16_t)(a - t);
  return ((uint32_t)hi << 16) | lo;
}
