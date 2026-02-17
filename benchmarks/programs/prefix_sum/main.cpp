#include <cstdio>
#include <cstdlib>
#define MOD 1000000007L

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *arr = new long[n];
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        arr[i] = ((seed >> 16) & 0x7fff) % 1000;
    }
    for (long i = 1; i < n; i++) arr[i] = (arr[i] + arr[i-1]) % MOD;
    printf("%ld\n", arr[n-1]);
    delete[] arr;
    return 0;
}
