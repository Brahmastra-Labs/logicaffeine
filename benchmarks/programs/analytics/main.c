// analytics — a realistic filter → map → group-by → reduce pipeline, the shape of a
// SQL `GROUP BY` or a pandas `groupby().agg()`: over N generated records, drop some,
// transform the rest, aggregate a sum per key, then reduce to a checksum. Deterministic
// input via the shared LCG; two-line output (kept count, checksum), byte-identical across
// every language. A composition of the real ETL kernels, not a single teaching algorithm.
#include <stdio.h>
#include <stdlib.h>

#define MOD 1000000007L
#define NGROUP 32768

int main(int argc, char **argv) {
    if (argc < 2) { fprintf(stderr, "usage: analytics <n>\n"); return 1; }
    long n = atol(argv[1]);
    long seed = 42;
    static long groups[NGROUP];
    for (long i = 0; i < NGROUP; i++) groups[i] = 0;
    long kept = 0;
    for (long r = 0; r < n; r++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        long key = (seed >> 16) & 0x7fff;
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        long value = (seed >> 16) & 0x7fff;
        if (value % 3 == 0) continue;                  // FILTER
        long mapped = (value * key + 7) % MOD;          // MAP
        long g = key % NGROUP;                          // GROUP BY key
        groups[g] = (groups[g] + mapped) % MOD;         // aggregate SUM
        kept++;
    }
    long checksum = 0;                                  // REDUCE
    for (long g = 0; g < NGROUP; g++)
        checksum = (checksum * 31 + groups[g]) & 268435455;
    printf("%ld\n%ld\n", kept, checksum);
    return 0;
}
