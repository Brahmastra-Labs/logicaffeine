#include <cstdio>
#include <cstdlib>

long gcd(long a, long b) {
    while (b > 0) { long t = b; b = a % b; a = t; }
    return a;
}

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: gcd <n>\n"); return 1; }
    long n = atol(argv[1]);
    long sum = 0;
    for (long i = 1; i <= n; i++)
        for (long j = i; j <= n; j++)
            sum += gcd(i, j);
    printf("%ld\n", sum);
    return 0;
}
