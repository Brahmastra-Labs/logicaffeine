/* K2 horner — ((2x+5)x-3)x+100, the residual of fabric_rtl::POLY_BODY, in uint64_t so it wraps as the
   64-bit fabric does. */
#include <stdint.h>

int64_t poly(int64_t x) {
  uint64_t u = (uint64_t)x;
  return (int64_t)(((2 * u + 5) * u - 3) * u + 100);
}
