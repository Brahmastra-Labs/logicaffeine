import sys

def sift_down(arr, start, end):
    root = start
    while 2 * root + 1 <= end:
        child = 2 * root + 1
        swap = root
        if arr[swap] < arr[child]: swap = child
        if child + 1 <= end and arr[swap] < arr[child + 1]: swap = child + 1
        if swap == root: return
        arr[root], arr[swap] = arr[swap], arr[root]
        root = swap

n = int(sys.argv[1])
arr = []
seed = 42
for _ in range(n):
    seed = (seed * 1103515245 + 12345) % 2147483648
    arr.append((seed >> 16) & 0x7fff)
for s in range((n - 2) // 2, -1, -1): sift_down(arr, s, n - 1)
for end in range(n - 1, 0, -1):
    arr[0], arr[end] = arr[end], arr[0]
    sift_down(arr, 0, end - 1)
checksum = 0
for v in arr: checksum = (checksum + v) % 1000000007
print(f"{arr[0]} {arr[-1]} {checksum}")
