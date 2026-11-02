/* K6 through block-RAM interfaces: the four lane operands read from one array and the four lane results
   written to another, the natural C interface of the step's array arguments. */
#include "kernel.c"
#pragma HLS interface port = x mode = array elem_count = 4
#pragma HLS interface port = y mode = array elem_count = 4
#pragma HLS interface port = sum mode = none
void simt_arrays(int64_t a, int64_t b, uint8_t mask, const int64_t x[4], int64_t y[4], int64_t *sum) {
  simt_axpb_sum(a, b, mask, x, y, sum);
}
