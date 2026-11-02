/* K4 through block-RAM interfaces: the state read from one 25-word array and written to another, the natural
   C interface of an array argument. The permutation is kernel.c's, on a local copy. */
#include "kernel.c"
#pragma HLS interface port = in mode = array elem_count = 25
#pragma HLS interface port = out mode = array elem_count = 25
void keccak_arrays(const uint64_t in[25], uint64_t out[25]) {
  uint64_t st[25];
  for (int i = 0; i < 25; i++) st[i] = in[i];
  keccak_f1600(st);
  for (int i = 0; i < 25; i++) out[i] = st[i];
}
