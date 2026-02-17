#include <cstdio>
#include <cstdlib>
#define MOD 1000000007L

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *arr = new long[n];
    for (long i = 0; i < n; i++) arr[i] = (i * 7 + 3) % 1000000;
    long sum = 0;
    for (long i = 0; i < n; i++) sum = (sum + arr[i]) % MOD;
    printf("%ld\n", sum);
    delete[] arr;
    return 0;
}
