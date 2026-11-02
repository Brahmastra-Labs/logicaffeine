#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: sieve <limit>\n"); return 1; }
    int limit = atoi(argv[1]);
    char *sieve = calloc(limit + 1, 1);
    if (!sieve) return 1;
    long count = 0;
    long h = 0;
    for (int i = 2; i <= limit; i++) {
        h = (h * 31 + !sieve[i]) & 268435455;
        if (!sieve[i]) {
            count++;
            for (long j = (long)i * i; j <= limit; j += i)
                sieve[j] = 1;
        }
    }
    printf("%ld %ld\n", count, h);
    free(sieve);
    return 0;
}
