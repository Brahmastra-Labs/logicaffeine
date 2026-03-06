#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define MOD 1000000007L

void merge_sort(long *arr, long n) {
    if (n < 2) return;
    long mid = n / 2;
    long *left = malloc(mid * sizeof(long));
    long *right = malloc((n - mid) * sizeof(long));
    memcpy(left, arr, mid * sizeof(long));
    memcpy(right, arr + mid, (n - mid) * sizeof(long));
    merge_sort(left, mid);
    merge_sort(right, n - mid);
    long i = 0, j = 0, k = 0;
    while (i < mid && j < n - mid) {
        if (left[i] <= right[j]) arr[k++] = left[i++];
        else arr[k++] = right[j++];
    }
    while (i < mid) arr[k++] = left[i++];
    while (j < n - mid) arr[k++] = right[j++];
    free(left);
    free(right);
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
    merge_sort(arr, n);
    long checksum = 0;
    for (long i = 0; i < n; i++) checksum = (checksum + arr[i]) % MOD;
    printf("%ld %ld %ld\n", arr[0], arr[n-1], checksum);
    free(arr);
    return 0;
}
