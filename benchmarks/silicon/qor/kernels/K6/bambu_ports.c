/* K6 through parallel ports: every lane operand in as a scalar and every result out through an `ovalid` port (each value
   marked by its own one-cycle strobe), the shape of a SIMT step's register file. The step is kernel.c's. */
#include "kernel.c"
#pragma HLS interface port = y0 mode = ovalid
#pragma HLS interface port = y1 mode = ovalid
#pragma HLS interface port = y2 mode = ovalid
#pragma HLS interface port = y3 mode = ovalid
#pragma HLS interface port = sum mode = ovalid
void simt_ports(int64_t a, int64_t b, uint8_t mask, int64_t x0, int64_t x1, int64_t x2, int64_t x3,
                int64_t *y0, int64_t *y1, int64_t *y2, int64_t *y3, int64_t *sum) {
  int64_t x[4] = {x0, x1, x2, x3};
  int64_t y[4];
  simt_axpb_sum(a, b, mask, x, y, sum);
  *y0 = y[0]; *y1 = y[1]; *y2 = y[2]; *y3 = y[3];
}
