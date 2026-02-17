#include <stdio.h>
#include <stdlib.h>

int count;
int n;

void solve(int row, int cols, int diag1, int diag2) {
    if (row == n) { count++; return; }
    int available = ((1 << n) - 1) & ~(cols | diag1 | diag2);
    while (available) {
        int bit = available & (-available);
        available ^= bit;
        solve(row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1);
    }
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    n = atoi(argv[1]);
    count = 0;
    solve(0, 0, 0, 0);
    printf("%d\n", count);
    return 0;
}
