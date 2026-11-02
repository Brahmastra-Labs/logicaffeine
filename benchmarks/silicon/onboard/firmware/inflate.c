// inflate — a raw DEFLATE decoder (RFC 1951) for the boot ROM: the compressed boot image streams in from the flash a
// byte at a time and inflates straight into the DDR2, every back-reference reaching into what is already written
// there. Stored, fixed-Huffman and dynamic-Huffman blocks. Codes of up to FAST_BITS bits are decoded with one table
// lookup; longer ones walk the canonical code. A malformed stream, or output that would run past the buffer, is
// refused. Its tables are the caller's, so it needs almost nothing of the boot ROM's scratch RAM.

#include "boot.h"

#define FAST_BITS INFLATE_FAST_BITS

struct bits {
  uint8_t (*next)(void);
  uint64_t taken, limit;  // bytes of the stream consumed, and its length
  uint64_t buf;
  uint32_t count;
  uint32_t padded;        // zero bytes fed past the stream's end, so a lookahead never reads outside it
};

static inline void fill(struct bits *b, uint32_t n) {
  while (b->count < n) {
    uint64_t byte;
    if (b->taken < b->limit) {
      byte = b->next();
      b->taken++;
    } else {
      byte = 0;
      b->padded++;
    }
    b->buf |= byte << b->count;
    b->count += 8;
  }
}

static inline uint32_t take(struct bits *b, uint32_t n) {
  if (n == 0) return 0;
  fill(b, n);
  uint32_t v = (uint32_t)(b->buf & ((1ull << n) - 1));
  b->buf >>= n;
  b->count -= n;
  return v;
}

static uint32_t reverse(uint32_t code, uint32_t len) {
  uint32_t r = 0;
  for (uint32_t i = 0; i < len; i++) {
    r = (r << 1) | (code & 1);
    code >>= 1;
  }
  return r;
}

static int build(struct huff *h, const uint8_t *lengths, uint32_t n) {
  uint16_t offs[16];
  for (int i = 0; i < 16; i++) h->counts[i] = 0;
  for (uint32_t i = 0; i < n; i++) h->counts[lengths[i]]++;
  h->counts[0] = 0;
  int left = 1;
  for (int len = 1; len < 16; len++) {
    left <<= 1;
    left -= h->counts[len];
    if (left < 0) return 0;
  }
  offs[1] = 0;
  for (int len = 1; len < 15; len++) offs[len + 1] = offs[len] + h->counts[len];
  for (uint32_t i = 0; i < n; i++)
    if (lengths[i]) h->symbols[offs[lengths[i]]++] = (uint16_t)i;
  // The fast table: every code of length <= FAST_BITS, bit-reversed (the stream holds a code's first bit lowest), at
  // every index whose low bits are that code.
  for (int i = 0; i < (1 << FAST_BITS); i++) h->fast_len[i] = 0;
  uint32_t code = 0, index = 0;
  for (uint32_t len = 1; len <= FAST_BITS; len++) {
    for (uint32_t k = 0; k < h->counts[len]; k++) {
      for (uint32_t at = reverse(code + k, len); at < (1u << FAST_BITS); at += 1u << len) {
        h->fast_sym[at] = h->symbols[index + k];
        h->fast_len[at] = (uint8_t)len;
      }
    }
    index += h->counts[len];
    code = (code + h->counts[len]) << 1;
  }
  return 1;
}

static int decode(struct bits *b, const struct huff *h) {
  fill(b, 16);
  uint32_t peek = (uint32_t)(b->buf & ((1u << FAST_BITS) - 1));
  uint32_t len = h->fast_len[peek];
  if (len) {
    b->buf >>= len;
    b->count -= len;
    return h->fast_sym[peek];
  }
  int code = 0, first = 0, index = 0;
  for (int l = 1; l < 16; l++) {
    code |= (int)take(b, 1);
    int count = h->counts[l];
    if (code - count < first) return h->symbols[index + (code - first)];
    index += count;
    first += count;
    first <<= 1;
    code <<= 1;
  }
  return -1;
}

static const uint16_t len_base[29] = {3,  4,  5,  6,  7,  8,  9,  10, 11,  13,  15,  17,  19,  23, 27,
                                      31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258};
static const uint8_t len_extra[29] = {0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0};
static const uint16_t dist_base[30] = {1,   2,   3,   4,   5,   7,    9,    13,   17,   25,   33,   49,   65,    97,    129,
                                       193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577};
static const uint8_t dist_extra[30] = {0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13};

static int codes(struct bits *b, const struct huff *lit, const struct huff *dist, uint8_t *out, uint64_t cap, uint64_t *pos) {
  uint64_t p = *pos;
  for (;;) {
    int sym = decode(b, lit);
    if (sym < 0) return 0;
    if (sym < 256) {
      if (p >= cap) return 0;
      out[p++] = (uint8_t)sym;
    } else if (sym == 256) {
      *pos = p;
      return 1;
    } else {
      sym -= 257;
      if (sym >= 29) return 0;
      uint32_t len = len_base[sym] + take(b, len_extra[sym]);
      int d = decode(b, dist);
      if (d < 0 || d >= 30) return 0;
      uint64_t back = dist_base[d] + take(b, dist_extra[d]);
      if (back > p || p + len > cap) return 0;
      const uint8_t *from = out + p - back;
      uint8_t *to = out + p;
      for (uint32_t i = 0; i < len; i++) to[i] = from[i];
      p += len;
    }
    if (b->padded > 8) return 0;
  }
}

static int fixed(struct inflate_tables *t, struct bits *b, uint8_t *out, uint64_t cap, uint64_t *pos) {
  uint8_t lengths[288];
  for (int i = 0; i < 144; i++) lengths[i] = 8;
  for (int i = 144; i < 256; i++) lengths[i] = 9;
  for (int i = 256; i < 280; i++) lengths[i] = 7;
  for (int i = 280; i < 288; i++) lengths[i] = 8;
  build(&t->lit, lengths, 288);
  for (int i = 0; i < 30; i++) lengths[i] = 5;
  build(&t->dist, lengths, 30);
  return codes(b, &t->lit, &t->dist, out, cap, pos);
}

static int dynamic(struct inflate_tables *t, struct bits *b, uint8_t *out, uint64_t cap, uint64_t *pos) {
  static const uint8_t order[19] = {16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15};
  uint8_t lengths[320];
  uint32_t nlen = take(b, 5) + 257, ndist = take(b, 5) + 1, ncode = take(b, 4) + 4;
  if (nlen > 286 || ndist > 30) return 0;
  for (uint32_t i = 0; i < 19; i++) lengths[order[i]] = i < ncode ? (uint8_t)take(b, 3) : 0;
  if (!build(&t->lencode, lengths, 19)) return 0;
  uint32_t i = 0;
  while (i < nlen + ndist) {
    int sym = decode(b, &t->lencode);
    if (sym < 0) return 0;
    if (sym < 16) {
      lengths[i++] = (uint8_t)sym;
    } else {
      uint8_t rep = 0;
      uint32_t times;
      if (sym == 16) {
        if (i == 0) return 0;
        rep = lengths[i - 1];
        times = 3 + take(b, 2);
      } else if (sym == 17) {
        times = 3 + take(b, 3);
      } else {
        times = 11 + take(b, 7);
      }
      if (i + times > nlen + ndist) return 0;
      while (times--) lengths[i++] = rep;
    }
  }
  if (lengths[256] == 0) return 0;
  if (!build(&t->lit, lengths, nlen) || !build(&t->dist, lengths + nlen, ndist)) return 0;
  return codes(b, &t->lit, &t->dist, out, cap, pos);
}

// Inflate `packed` bytes drawn from `next` into `out` (room for `cap`), building codes in `t`; the inflated length, or
// -1. Every byte of the stream is drawn, so a digest of what `next` returns covers all of it.
int64_t inflate(struct inflate_tables *t, uint8_t (*next)(void), uint64_t packed, uint8_t *out, uint64_t cap) {
  struct bits b = {next, 0, packed, 0, 0, 0};
  uint64_t pos = 0;
  int last;
  do {
    last = (int)take(&b, 1);
    uint32_t type = take(&b, 2);
    if (type == 0) {
      uint32_t drop = b.count % 8;
      b.buf >>= drop;
      b.count -= drop;
      uint32_t len = take(&b, 16), nlen = take(&b, 16);
      if (len != (~nlen & 0xffff) || pos + len > cap) return -1;
      for (uint32_t i = 0; i < len; i++) out[pos++] = (uint8_t)take(&b, 8);
    } else if (type == 1) {
      if (!fixed(t, &b, out, cap, &pos)) return -1;
    } else if (type == 2) {
      if (!dynamic(t, &b, out, cap, &pos)) return -1;
    } else {
      return -1;
    }
    if (b.padded > 8) return -1;
  } while (!last);
  while (b.taken < b.limit) {
    b.next();
    b.taken++;
  }
  return (int64_t)pos;
}
