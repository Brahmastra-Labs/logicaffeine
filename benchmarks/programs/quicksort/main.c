#include <stdio.h>
#include <stdlib.h>
#define MOD 1000000007L

void swap(long *a, long *b) { long t = *a; *a = *b; *b = t; }

long partition(long *arr, long lo, long hi) {
    long pivot = arr[hi];
    long i = lo;
    for (long j = lo; j < hi; j++) {
        if (arr[j] <= pivot) { swap(&arr[i], &arr[j]); i++; }
    }
    swap(&arr[i], &arr[hi]);
    return i;
}

void qs(long *arr, long lo, long hi) {
    if (lo < hi) {
        long p = partition(arr, lo, hi);
        qs(arr, lo, p - 1);
        qs(arr, p + 1, hi);
    }
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *arr = malloc(n * sizeof(long));
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        arr[i] = (seed >> 16) & 0x7fff;
    }
    qs(arr, 0, n - 1);
    long checksum = 0;
    for (long i = 0; i < n; i++) checksum = (checksum + arr[i]) % MOD;
    printf("%ld %ld %ld\n", arr[0], arr[n-1], checksum);
    free(arr);
    return 0;
}
