#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long *arr = malloc(n * sizeof(long));
    for (long i = 0; i < n; i++) arr[i] = (i * 7 + 3) % 1000000;
    long h = 0;
    for (long i = 0; i < n; i++) h = (h * 31 + arr[i]) & 268435455;
    printf("%ld\n", h);
    free(arr);
    return 0;
}
