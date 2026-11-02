/* K3 gemm8 — one row of eight int8 activations times an 8x8 int8 weight tile, int32 sums:
   o[c] = sum over r of a[r] * w[r][c]. Every sum fits int32 (at most 8 * 128 * 128). gemm8_tile runs
   eight rows against one weight tile, the call the weight-stationary contract reloads weights for. */
#include <stdint.h>

void gemm8_row(const int8_t a[8], const int8_t w[8][8], int32_t o[8]) {
  for (int c = 0; c < 8; c++) {
    int32_t acc = 0;
    for (int r = 0; r < 8; r++)
      acc += (int32_t)a[r] * (int32_t)w[r][c];
    o[c] = acc;
  }
}

void gemm8_tile(const int8_t a[8][8], const int8_t w[8][8], int32_t o[8][8]) {
  for (int i = 0; i < 8; i++)
    gemm8_row(a[i], w, o[i]);
}
