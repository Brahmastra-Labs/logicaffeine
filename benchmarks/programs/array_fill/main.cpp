#include <cstdio>
#include <cstdlib>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *arr = new long[n];
    for (long i = 0; i < n; i++) arr[i] = (i * 7 + 3) % 1000000;
    long h = 0;
    for (long i = 0; i < n; i++) h = (h * 31 + arr[i]) & 268435455;
    printf("%ld\n", h);
    delete[] arr;
    return 0;
}
