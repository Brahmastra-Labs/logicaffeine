#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: collatz <n>\n"); return 1; }
    long n = atol(argv[1]);
    long total = 0;
    for (long i = 1; i <= n; i++) {
        long k = i;
        while (k != 1) {
            if (k % 2 == 0) k = k / 2;
            else k = 3 * k + 1;
            total++;
        }
    }
    printf("%ld\n", total);
    return 0;
}
