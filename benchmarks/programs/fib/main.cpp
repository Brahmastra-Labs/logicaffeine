#include <cstdio>
#include <cstdlib>

long fib(long n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: fib <n>\n"); return 1; }
    long n = std::atol(argv[1]);
    printf("%ld\n", fib(n));
    return 0;
}
