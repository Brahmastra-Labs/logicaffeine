import sys
sys.setrecursionlimit(1000000)

def merge_sort(arr):
    n = len(arr)
    if n < 2: return
    mid = n // 2
    left = arr[:mid]
    right = arr[mid:]
    merge_sort(left)
    merge_sort(right)
    i = j = k = 0
    while i < mid and j < n - mid:
        if left[i] <= right[j]: arr[k] = left[i]; i += 1
        else: arr[k] = right[j]; j += 1
        k += 1
    while i < mid: arr[k] = left[i]; i += 1; k += 1
    while j < n - mid: arr[k] = right[j]; j += 1; k += 1


def main():
    n = int(sys.argv[1])
    arr = []
    seed = 42
    for _ in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        arr.append((seed >> 16) & 0x7fff)
    merge_sort(arr)
    checksum = 0
    for v in arr: checksum = (checksum * 31 + v) & 268435455
    print(f"{arr[0]} {arr[-1]} {checksum}")


if __name__ == "__main__":
    main()
