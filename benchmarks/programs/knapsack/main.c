#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long capacity = n * 5;
    long *weight = malloc(n * sizeof(long));
    long *value = malloc(n * sizeof(long));
    for (long i = 0; i < n; i++) {
        weight[i] = (i * 17 + 3) % 50 + 1;
        value[i] = (i * 31 + 7) % 100 + 1;
    }
    long cols = capacity + 1;
    long *prev = calloc(cols, sizeof(long));
    long *curr = calloc(cols, sizeof(long));
    for (long i = 0; i < n; i++) {
        for (long w = 0; w <= capacity; w++) {
            curr[w] = prev[w];
            if (w >= weight[i] && prev[w - weight[i]] + value[i] > curr[w])
                curr[w] = prev[w - weight[i]] + value[i];
        }
        long *tmp = prev; prev = curr; curr = tmp;
    }
    printf("%ld\n", prev[capacity]);
    free(weight); free(value); free(prev); free(curr);
    return 0;
}
