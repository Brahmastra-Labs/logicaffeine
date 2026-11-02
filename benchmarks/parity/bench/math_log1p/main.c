#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long s = 0;
    for (long i = 1; i <= n; i++) { s += (long)(log1p((double)i/1000.0) * 1000000.0); }
    printf("%ld\n", s);
    return 0;
}
