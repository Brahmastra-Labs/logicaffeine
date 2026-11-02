/* K4 through parallel ports: the 25 lanes in as scalars and out through `ovalid` ports (each value marked by
   its own one-cycle strobe), the shape of the LOGOS keccak_core's 1600-bit buses. The permutation is kernel.c's. */
#include "kernel.c"
#pragma HLS interface port = o0 mode = ovalid
#pragma HLS interface port = o1 mode = ovalid
#pragma HLS interface port = o2 mode = ovalid
#pragma HLS interface port = o3 mode = ovalid
#pragma HLS interface port = o4 mode = ovalid
#pragma HLS interface port = o5 mode = ovalid
#pragma HLS interface port = o6 mode = ovalid
#pragma HLS interface port = o7 mode = ovalid
#pragma HLS interface port = o8 mode = ovalid
#pragma HLS interface port = o9 mode = ovalid
#pragma HLS interface port = o10 mode = ovalid
#pragma HLS interface port = o11 mode = ovalid
#pragma HLS interface port = o12 mode = ovalid
#pragma HLS interface port = o13 mode = ovalid
#pragma HLS interface port = o14 mode = ovalid
#pragma HLS interface port = o15 mode = ovalid
#pragma HLS interface port = o16 mode = ovalid
#pragma HLS interface port = o17 mode = ovalid
#pragma HLS interface port = o18 mode = ovalid
#pragma HLS interface port = o19 mode = ovalid
#pragma HLS interface port = o20 mode = ovalid
#pragma HLS interface port = o21 mode = ovalid
#pragma HLS interface port = o22 mode = ovalid
#pragma HLS interface port = o23 mode = ovalid
#pragma HLS interface port = o24 mode = ovalid
void keccak_ports(uint64_t i0, uint64_t i1, uint64_t i2, uint64_t i3, uint64_t i4, uint64_t i5, uint64_t i6,
                  uint64_t i7, uint64_t i8, uint64_t i9, uint64_t i10, uint64_t i11, uint64_t i12, uint64_t i13,
                  uint64_t i14, uint64_t i15, uint64_t i16, uint64_t i17, uint64_t i18, uint64_t i19, uint64_t i20,
                  uint64_t i21, uint64_t i22, uint64_t i23, uint64_t i24,
                  uint64_t *o0, uint64_t *o1, uint64_t *o2, uint64_t *o3, uint64_t *o4, uint64_t *o5, uint64_t *o6,
                  uint64_t *o7, uint64_t *o8, uint64_t *o9, uint64_t *o10, uint64_t *o11, uint64_t *o12,
                  uint64_t *o13, uint64_t *o14, uint64_t *o15, uint64_t *o16, uint64_t *o17, uint64_t *o18,
                  uint64_t *o19, uint64_t *o20, uint64_t *o21, uint64_t *o22, uint64_t *o23, uint64_t *o24) {
  uint64_t st[25] = {i0, i1, i2, i3, i4, i5, i6, i7, i8, i9, i10, i11, i12,
                     i13, i14, i15, i16, i17, i18, i19, i20, i21, i22, i23, i24};
  keccak_f1600(st);
  *o0 = st[0]; *o1 = st[1]; *o2 = st[2]; *o3 = st[3]; *o4 = st[4]; *o5 = st[5]; *o6 = st[6];
  *o7 = st[7]; *o8 = st[8]; *o9 = st[9]; *o10 = st[10]; *o11 = st[11]; *o12 = st[12]; *o13 = st[13];
  *o14 = st[14]; *o15 = st[15]; *o16 = st[16]; *o17 = st[17]; *o18 = st[18]; *o19 = st[19];
  *o20 = st[20]; *o21 = st[21]; *o22 = st[22]; *o23 = st[23]; *o24 = st[24];
}
