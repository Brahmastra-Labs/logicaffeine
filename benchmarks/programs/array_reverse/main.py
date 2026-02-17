import sys
n = int(sys.argv[1])
arr = []
seed = 42
for _ in range(n):
    seed = (seed * 1103515245 + 12345) % 2147483648
    arr.append((seed >> 16) & 0x7fff)
lo, hi = 0, n - 1
while lo < hi:
    arr[lo], arr[hi] = arr[hi], arr[lo]
    lo += 1; hi -= 1
print(f"{arr[0]} {arr[-1]} {arr[n // 2]}")
