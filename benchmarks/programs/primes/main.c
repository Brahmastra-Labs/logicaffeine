#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: primes <n>\n"); return 1; }
    long n = atol(argv[1]);
    long count = 0;
    for (long i = 2; i <= n; i++) {
        int is_prime = 1;
        for (long d = 2; d * d <= i; d++) {
            if (i % d == 0) { is_prime = 0; break; }
        }
        if (is_prime) count++;
    }
    printf("%ld\n", count);
    return 0;
}
