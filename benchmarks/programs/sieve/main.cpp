#include <cstdio>
#include <cstdlib>
#include <vector>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: sieve <limit>\n"); return 1; }
    int limit = std::atoi(argv[1]);
    std::vector<char> sieve(limit + 1, 0);
    int count = 0;
    for (int i = 2; i <= limit; i++) {
        if (!sieve[i]) {
            count++;
            for (long j = (long)i * i; j <= limit; j += i)
                sieve[j] = true;
        }
    }
    printf("%d\n", count);
    return 0;
}
