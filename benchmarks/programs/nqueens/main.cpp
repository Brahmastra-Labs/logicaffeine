#include <cstdio>
#include <cstdlib>

long solve(long n, long row, long cols, long diag1, long diag2) {
    if (row == n) return 1;
    long count = 0;
    long available = ((1L << n) - 1) & ~(cols | diag1 | diag2);
    while (available) {
        long bit = available & (-available);
        available ^= bit;
        count += solve(n, row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1);
    }
    return count;
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = std::atol(argv[1]);
    long count = solve(n, 0, 0, 0, 0);
    printf("%ld\n", count);
    return 0;
}
