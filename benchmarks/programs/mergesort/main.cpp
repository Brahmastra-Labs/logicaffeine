#include <cstdio>
#include <cstdlib>
#define MOD 1000000007L

void merge(long *arr, long *tmp, long lo, long mid, long hi) {
    long i = lo, j = mid, k = lo;
    while (i < mid && j < hi) {
        if (arr[i] <= arr[j]) tmp[k++] = arr[i++];
        else tmp[k++] = arr[j++];
    }
    while (i < mid) tmp[k++] = arr[i++];
    while (j < hi) tmp[k++] = arr[j++];
    for (long x = lo; x < hi; x++) arr[x] = tmp[x];
}

void mergesort(long *arr, long *tmp, long lo, long hi) {
    if (hi - lo < 2) return;
    long mid = lo + (hi - lo) / 2;
    mergesort(arr, tmp, lo, mid);
    mergesort(arr, tmp, mid, hi);
    merge(arr, tmp, lo, mid, hi);
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *arr = new long[n];
    long *tmp = new long[n];
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        arr[i] = (seed >> 16) & 0x7fff;
    }
    mergesort(arr, tmp, 0, n);
    long checksum = 0;
    for (long i = 0; i < n; i++) checksum = (checksum + arr[i]) % MOD;
    printf("%ld %ld %ld\n", arr[0], arr[n-1], checksum);
    delete[] arr; delete[] tmp;
    return 0;
}
