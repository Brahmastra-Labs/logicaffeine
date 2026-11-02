/* K3 through block-RAM interfaces, one tile per call: eight activation rows, the weight tile and the eight
   rows of sums as arrays, row-major — the natural C interface of kernel.c's gemm8_tile. */
#include "kernel.c"
#pragma HLS interface port = a mode = array elem_count = 64
#pragma HLS interface port = w mode = array elem_count = 64
#pragma HLS interface port = o mode = array elem_count = 64
void gemm8_arrays(const int8_t a[64], const int8_t w[64], int32_t o[64]) {
  gemm8_tile((const int8_t(*)[8])a, (const int8_t(*)[8])w, (int32_t(*)[8])o);
}
