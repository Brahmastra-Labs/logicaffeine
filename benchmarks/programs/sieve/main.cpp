#include <cstdio>
#include <cstdlib>
#include <vector>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: sieve <limit>\n"); return 1; }
    int limit = std::atoi(argv[1]);
    std::vector<char> sieve(limit + 1, 0);
    long count = 0;
    long h = 0;
    for (int i = 2; i <= limit; i++) {
        h = (h * 31 + !sieve[i]) & 268435455;
        if (!sieve[i]) {
            count++;
            for (long j = (long)i * i; j <= limit; j += i)
                sieve[j] = true;
        }
    }
    printf("%ld %ld\n", count, h);
    return 0;
}
