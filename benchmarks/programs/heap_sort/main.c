#include <stdio.h>
#include <stdlib.h>
#define MOD 1000000007L

void sift_down(long *arr, long start, long end) {
    long root = start;
    while (2 * root + 1 <= end) {
        long child = 2 * root + 1;
        long swap_idx = root;
        if (arr[swap_idx] < arr[child]) swap_idx = child;
        if (child + 1 <= end && arr[swap_idx] < arr[child + 1]) swap_idx = child + 1;
        if (swap_idx == root) return;
        long tmp = arr[root]; arr[root] = arr[swap_idx]; arr[swap_idx] = tmp;
        root = swap_idx;
    }
}

void heap_sort(long *arr, long n) {
    for (long start = (n - 2) / 2; start >= 0; start--)
        sift_down(arr, start, n - 1);
    for (long end = n - 1; end > 0; end--) {
        long tmp = arr[0]; arr[0] = arr[end]; arr[end] = tmp;
        sift_down(arr, 0, end - 1);
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
    heap_sort(arr, n);
    long checksum = 0;
    for (long i = 0; i < n; i++) checksum = (checksum + arr[i]) % MOD;
    printf("%ld %ld %ld\n", arr[0], arr[n-1], checksum);
    free(arr);
    return 0;
}
