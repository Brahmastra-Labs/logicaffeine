#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long s = 0;
    for (long i = 1; i <= n; i++) { char b[24]; sprintf(b, "%ld", i); s += (long)atof(b); }
    printf("%ld\n", s);
    return 0;
}
