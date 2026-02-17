import sys
sys.setrecursionlimit(1000000)

def merge_sort(arr):
    if len(arr) < 2: return arr
    mid = len(arr) // 2
    left = merge_sort(arr[:mid])
    right = merge_sort(arr[mid:])
    result = []
    i = j = 0
    while i < len(left) and j < len(right):
        if left[i] <= right[j]: result.append(left[i]); i += 1
        else: result.append(right[j]); j += 1
    result.extend(left[i:]); result.extend(right[j:])
    return result

n = int(sys.argv[1])
arr = []
seed = 42
for _ in range(n):
    seed = (seed * 1103515245 + 12345) % 2147483648
    arr.append((seed >> 16) & 0x7fff)
arr = merge_sort(arr)
checksum = 0
for v in arr: checksum = (checksum + v) % 1000000007
print(f"{arr[0]} {arr[-1]} {checksum}")
