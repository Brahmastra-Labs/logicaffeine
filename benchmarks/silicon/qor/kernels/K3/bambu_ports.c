/* K3 through parallel ports, one row per call: the row's eight activations packed in one 64-bit word
   (byte r is a[r]), weight row r in word w<r> (byte c is w[r][c]), and the eight sums out through `ovalid`
   ports. The arithmetic is kernel.c's gemm8_row. */
#include "kernel.c"
#pragma HLS interface port = o0 mode = ovalid
#pragma HLS interface port = o1 mode = ovalid
#pragma HLS interface port = o2 mode = ovalid
#pragma HLS interface port = o3 mode = ovalid
#pragma HLS interface port = o4 mode = ovalid
#pragma HLS interface port = o5 mode = ovalid
#pragma HLS interface port = o6 mode = ovalid
#pragma HLS interface port = o7 mode = ovalid
void gemm8_ports(uint64_t a, uint64_t w0, uint64_t w1, uint64_t w2, uint64_t w3, uint64_t w4, uint64_t w5,
                 uint64_t w6, uint64_t w7, int32_t *o0, int32_t *o1, int32_t *o2, int32_t *o3, int32_t *o4,
                 int32_t *o5, int32_t *o6, int32_t *o7) {
  const uint64_t wr[8] = {w0, w1, w2, w3, w4, w5, w6, w7};
  int8_t av[8], wv[8][8];
  int32_t o[8];
  for (int r = 0; r < 8; r++) {
    av[r] = (int8_t)(uint8_t)(a >> (8 * r));
    for (int c = 0; c < 8; c++)
      wv[r][c] = (int8_t)(uint8_t)(wr[r] >> (8 * c));
  }
  gemm8_row(av, wv, o);
  *o0 = o[0];
  *o1 = o[1];
  *o2 = o[2];
  *o3 = o[3];
  *o4 = o[4];
  *o5 = o[5];
  *o6 = o[6];
  *o7 = o[7];
}
