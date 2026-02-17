import sys
sys.setrecursionlimit(1000000)

def qs(arr, lo, hi):
    if lo >= hi: return
    pivot = arr[hi]; i = lo
    for j in range(lo, hi):
        if arr[j] <= pivot: arr[i], arr[j] = arr[j], arr[i]; i += 1
    arr[i], arr[hi] = arr[hi], arr[i]
    qs(arr, lo, i - 1)
    qs(arr, i + 1, hi)

n = int(sys.argv[1])
arr = []
seed = 42
for _ in range(n):
    seed = (seed * 1103515245 + 12345) % 2147483648
    arr.append((seed >> 16) & 0x7fff)
qs(arr, 0, n - 1)
checksum = 0
for v in arr: checksum = (checksum + v) % 1000000007
print(f"{arr[0]} {arr[-1]} {checksum}")
