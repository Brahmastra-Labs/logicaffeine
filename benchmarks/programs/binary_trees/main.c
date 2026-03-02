#include <stdio.h>
#include <stdlib.h>

long make_check(int depth) {
    if (depth == 0) return 1;
    return 1 + make_check(depth - 1) + make_check(depth - 1);
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    int min_depth = 4;
    int max_depth = n;
    if (min_depth + 2 > max_depth) max_depth = min_depth + 2;

    printf("stretch tree of depth %d\t check: %ld\n", max_depth + 1, make_check(max_depth + 1));

    long long_lived = make_check(max_depth);

    for (int depth = min_depth; depth <= max_depth; depth += 2) {
        int iterations = 1 << (max_depth - depth + min_depth);
        long total_check = 0;
        for (int i = 0; i < iterations; i++) {
            total_check += make_check(depth);
        }
        printf("%d\t trees of depth %d\t check: %ld\n", iterations, depth, total_check);
    }
    printf("long lived tree of depth %d\t check: %ld\n", max_depth, long_lived);
    return 0;
}
