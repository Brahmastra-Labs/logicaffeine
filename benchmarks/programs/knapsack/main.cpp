#include <cstdio>
#include <cstdlib>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long capacity = n * 5;
    long *prev = new long[capacity + 1]();
    long *curr = new long[capacity + 1]();
    for (long i = 0; i < n; i++) {
        long w = (i * 17 + 3) % 50 + 1, v = (i * 31 + 7) % 100 + 1;
        for (long j = 0; j <= capacity; j++) {
            curr[j] = prev[j];
            if (j >= w && prev[j - w] + v > curr[j]) curr[j] = prev[j - w] + v;
        }
        long *t = prev; prev = curr; curr = t;
    }
    printf("%ld\n", prev[capacity]);
    delete[] prev; delete[] curr;
    return 0;
}
