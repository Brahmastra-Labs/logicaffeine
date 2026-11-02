#include <cstdio>
#include <cstdlib>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long capacity = n * 5;
    long *weight = new long[n];
    long *value = new long[n];
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        weight[i] = (seed >> 16) % 50 + 1;
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        value[i] = (seed >> 16) % 100 + 1;
    }
    long *prev = new long[capacity + 1]();
    long *curr = new long[capacity + 1]();
    for (long i = 0; i < n; i++) {
        long w = weight[i];
        long v = value[i];
        for (long j = 0; j <= capacity; j++) {
            curr[j] = prev[j];
            if (j >= w && prev[j - w] + v > curr[j]) curr[j] = prev[j - w] + v;
        }
        long *t = prev; prev = curr; curr = t;
    }
    long h = 0;
    for (long j = 0; j <= capacity; j++) h = (h * 31 + prev[j]) & 268435455;
    printf("%ld %ld\n", prev[capacity], h);
    delete[] weight; delete[] value; delete[] prev; delete[] curr;
    return 0;
}
