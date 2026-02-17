#include <cstdio>
#include <cstdlib>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: loop_sum <n>\n"); return 1; }
    long n = atol(argv[1]);
    long sum = 0;
    for (long i = 1; i <= n; i++) {
        sum = (sum + i) % 1000000007;
    }
    printf("%ld\n", sum);
    return 0;
}
