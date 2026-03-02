#include <stdio.h>
#include <stdlib.h>

int solve(int n, int row, int cols, int diag1, int diag2) {
    if (row == n) return 1;
    int count = 0;
    int available = ((1 << n) - 1) & ~(cols | diag1 | diag2);
    while (available) {
        int bit = available & (-available);
        available ^= bit;
        count += solve(n, row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1);
    }
    return count;
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    int count = solve(n, 0, 0, 0, 0);
    printf("%d\n", count);
    return 0;
}
