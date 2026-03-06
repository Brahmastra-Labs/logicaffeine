#include <cstdio>
#include <cstdlib>
#define MOD 1000000007L

long partition(long *arr, long lo, long hi) {
    long pivot = arr[hi], i = lo;
    for (long j = lo; j < hi; j++)
        if (arr[j] <= pivot) { long t = arr[i]; arr[i] = arr[j]; arr[j] = t; i++; }
    long t = arr[i]; arr[i] = arr[hi]; arr[hi] = t;
    return i;
}

void qs(long *arr, long lo, long hi) {
    if (lo < hi) { long p = partition(arr, lo, hi); qs(arr, lo, p-1); qs(arr, p+1, hi); }
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *arr = new long[n];
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        arr[i] = (seed >> 16) & 0x7fff;
    }
    qs(arr, 0, n - 1);
    long checksum = 0;
    for (long i = 0; i < n; i++) checksum = (checksum + arr[i]) % MOD;
    printf("%ld %ld %ld\n", arr[0], arr[n-1], checksum);
    delete[] arr;
    return 0;
}
