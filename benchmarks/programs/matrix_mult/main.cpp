#include <cstdio>
#include <cstdlib>
#define MOD 1000000007L

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *a = new long[n * n];
    long *b = new long[n * n];
    long *c = new long[n * n]();
    long seed = 42;
    for (long i = 0; i < n * n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        a[i] = (seed >> 16) % 100;
    }
    for (long i = 0; i < n * n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        b[i] = (seed >> 16) % 100;
    }
    for (long i = 0; i < n; i++)
        for (long k = 0; k < n; k++)
            for (long j = 0; j < n; j++)
                c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) % MOD;
    long checksum = 0;
    for (long i = 0; i < n * n; i++) checksum = (checksum * 31 + c[i]) & 268435455;
    printf("%ld\n", checksum);
    delete[] a; delete[] b; delete[] c;
    return 0;
}
