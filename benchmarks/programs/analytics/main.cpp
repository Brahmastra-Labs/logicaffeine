// analytics — filter → map → group-by → reduce pipeline (C++ port of main.c).
#include <cstdio>
#include <cstdlib>
#include <array>

constexpr long MOD = 1000000007L;
constexpr int NGROUP = 32768;

int main(int argc, char **argv) {
    if (argc < 2) { std::fprintf(stderr, "usage: analytics <n>\n"); return 1; }
    long n = std::atol(argv[1]);
    long seed = 42;
    static std::array<long, NGROUP> groups{};
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
    std::printf("%ld\n%ld\n", kept, checksum);
    return 0;
}
