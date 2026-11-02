#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long s = 0;
    for (long i = 1; i <= n; i++) { long x = i - n/2; s += (x < 0 ? -x : x); }
    printf("%ld\n", s);
    return 0;
}
