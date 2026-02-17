import sys
n = int(sys.argv[1])
arr = []
seed = 42
for _ in range(n):
    seed = (seed * 1103515245 + 12345) % 2147483648
    arr.append((seed >> 16) % 1000)
counts = [0] * 1000
for v in arr: counts[v] += 1
sorted_arr = []
for v in range(1000):
    sorted_arr.extend([v] * counts[v])
checksum = 0
for v in sorted_arr: checksum = (checksum + v) % 1000000007
print(f"{sorted_arr[0]} {sorted_arr[-1]} {checksum}")
