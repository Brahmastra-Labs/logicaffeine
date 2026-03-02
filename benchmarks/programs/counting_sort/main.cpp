#include <cstdio>
#include <cstdlib>
#include <cstring>
#define MOD 1000000007L

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *arr = new long[n];
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        arr[i] = (seed >> 16) % 1000;
    }
    long counts[1000] = {};
    for (long i = 0; i < n; i++) counts[arr[i]]++;
    long idx = 0;
    for (long v = 0; v < 1000; v++)
        for (long c = 0; c < counts[v]; c++)
            arr[idx++] = v;
    long checksum = 0;
    for (long i = 0; i < n; i++) checksum = (checksum + arr[i]) % MOD;
    printf("%ld %ld %ld\n", arr[0], arr[n-1], checksum);
    delete[] arr;
    return 0;
}
