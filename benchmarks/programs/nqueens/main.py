import sys

def solve(row, cols, diag1, diag2, n):
    if row == n: return 1
    count = 0
    available = ((1 << n) - 1) & ~(cols | diag1 | diag2)
    while available:
        bit = available & (-available)
        available ^= bit
        count += solve(row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1, n)
    return count

n = int(sys.argv[1])
print(solve(0, 0, 0, 0, n))
