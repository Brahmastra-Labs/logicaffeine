// The boot ROM's shared declarations.
#pragma once
#include <stddef.h>
#include <stdint.h>

void *memcpy(void *dst, const void *src, size_t n);
void *memset(void *dst, int c, size_t n);
int memcmp(const void *a, const void *b, size_t n);

// A canonical Huffman code: its codes by length (counts, symbols in canonical order), and a table indexed by the
// next INFLATE_FAST_BITS bits of the stream giving the symbol and its length for every code that short (length 0:
// longer).
#define INFLATE_FAST_BITS 9
struct huff {
  uint16_t counts[16];
  uint16_t symbols[288];
  uint16_t fast_sym[1 << INFLATE_FAST_BITS];
  uint8_t fast_len[1 << INFLATE_FAST_BITS];
};
// The inflater's working tables: the caller gives it room for them (the boot ROM, in the DDR2).
struct inflate_tables {
  struct huff lit, dist, lencode;
};
int64_t inflate(struct inflate_tables *t, uint8_t (*next)(void), uint64_t packed, uint8_t *out, uint64_t cap);
struct sha256 {
  uint32_t h[8];
  uint8_t buf[64];
  uint64_t len;
  uint32_t fill;
};
void sha256_init(struct sha256 *c);
void sha256_byte(struct sha256 *c, uint8_t b);
void sha256_final(struct sha256 *c, uint8_t out[32]);
