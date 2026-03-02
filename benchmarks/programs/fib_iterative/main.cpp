#include <cstdio>
#include <cstdlib>
#define MOD 1000000007L

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: fib_iterative <n>\n"); return 1; }
    long n = atol(argv[1]);
    long a = 0, b = 1;
    for (long i = 0; i < n; i++) {
        long temp = b;
        b = (a + b) % MOD;
        a = temp;
    }
    printf("%ld\n", a);
    return 0;
}
