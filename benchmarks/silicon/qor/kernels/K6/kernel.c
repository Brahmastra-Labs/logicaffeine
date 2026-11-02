/* K6 simt_axpb_sum — one step of the 4-lane SIMT engine (gpu_simt_rtl, shader `a * x + b`): every lane whose
   mask bit is set computes a*x+b (uint64_t, wrapping as the 64-bit lanes do), a masked lane yields 0, and the
   lanes sum. Results return through the y[] and sum outputs. */
#include <stdint.h>

void simt_axpb_sum(int64_t a, int64_t b, uint8_t mask, const int64_t x[4], int64_t y[4], int64_t *sum) {
  uint64_t acc = 0;
  for (int lane = 0; lane < 4; lane++) {
    uint64_t v = ((mask >> lane) & 1) ? (uint64_t)a * (uint64_t)x[lane] + (uint64_t)b : 0;
    y[lane] = (int64_t)v;
    acc += v;
  }
  *sum = (int64_t)acc;
}
