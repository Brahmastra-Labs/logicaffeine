#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long counts[1000];
    memset(counts, 0, sizeof(counts));
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        long v = ((seed >> 16) & 0x7fff) % 1000;
        counts[v]++;
    }
    long max_freq = 0, max_idx = 0, distinct = 0;
    for (long i = 0; i < 1000; i++) {
        if (counts[i] > 0) distinct++;
        if (counts[i] > max_freq) { max_freq = counts[i]; max_idx = i; }
    }
    printf("%ld %ld %ld\n", max_freq, max_idx, distinct);
    return 0;
}
