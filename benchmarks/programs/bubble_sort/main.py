import sys

n = int(sys.argv[1])
arr = [0] * n
seed = 42
for i in range(n):
    seed = (seed * 1103515245 + 12345) & 0xffffffff
    arr[i] = (seed >> 16) & 0x7fff
for i in range(n - 1):
    for j in range(n - 1 - i):
        if arr[j] > arr[j + 1]:
            arr[j], arr[j + 1] = arr[j + 1], arr[j]
print(arr[0])
