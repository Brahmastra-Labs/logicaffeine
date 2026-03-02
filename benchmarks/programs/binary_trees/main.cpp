#include <cstdio>
#include <cstdlib>

long make_check(int d) {
    if (d == 0) return 1;
    return 1 + make_check(d - 1) + make_check(d - 1);
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]), mn = 4, mx = n;
    if (mn + 2 > mx) mx = mn + 2;
    printf("stretch tree of depth %d\t check: %ld\n", mx + 1, make_check(mx + 1));
    long ll = make_check(mx);
    for (int d = mn; d <= mx; d += 2) {
        int it = 1 << (mx - d + mn);
        long tc = 0;
        for (int i = 0; i < it; i++) tc += make_check(d);
        printf("%d\t trees of depth %d\t check: %ld\n", it, d, tc);
    }
    printf("long lived tree of depth %d\t check: %ld\n", mx, ll);
    return 0;
}
