// The few C library functions a freestanding build needs, written so the compiler cannot turn them back into calls
// to themselves.

#include <stddef.h>
#include <stdint.h>

void *memcpy(void *dst, const void *src, size_t n) {
  uint8_t *d = dst;
  const uint8_t *s = src;
  if ((((uint64_t)d | (uint64_t)s) & 7) == 0) {
    while (n >= 8) {
      *(volatile uint64_t *)d = *(const uint64_t *)s;
      d += 8;
      s += 8;
      n -= 8;
    }
  }
  while (n--) *(volatile uint8_t *)d++ = *s++;
  return dst;
}

void *memset(void *dst, int c, size_t n) {
  uint8_t *d = dst;
  uint64_t w = (uint8_t)c * 0x0101010101010101ull;
  while (n && ((uint64_t)d & 7)) {
    *(volatile uint8_t *)d++ = (uint8_t)c;
    n--;
  }
  while (n >= 8) {
    *(volatile uint64_t *)d = w;
    d += 8;
    n -= 8;
  }
  while (n--) *(volatile uint8_t *)d++ = (uint8_t)c;
  return dst;
}

int memcmp(const void *a, const void *b, size_t n) {
  const uint8_t *x = a, *y = b;
  for (size_t i = 0; i < n; i++)
    if (x[i] != y[i]) return x[i] < y[i] ? -1 : 1;
  return 0;
}

// The one compiler-runtime helper the firmware needs (SoftFloat counts leading zeros through __builtin_clzll). The
// cross toolchain's libgcc is built for the hard-float ABI, which a hart with no floating-point unit cannot link.
int __clzdi2(uint64_t x) {
  int n = 0;
  if (!x) return 64;
  while (!(x >> 63)) {
    x <<= 1;
    n++;
  }
  return n;
}
