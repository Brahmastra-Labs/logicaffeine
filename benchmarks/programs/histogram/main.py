import sys
n = int(sys.argv[1])
counts = [0] * 1000
seed = 42
for _ in range(n):
    seed = (seed * 1103515245 + 12345) % 2147483648
    counts[((seed >> 16) & 0x7fff) % 1000] += 1
max_freq = max_idx = distinct = 0
for i in range(1000):
    if counts[i] > 0: distinct += 1
    if counts[i] > max_freq: max_freq = counts[i]; max_idx = i
print(f"{max_freq} {max_idx} {distinct}")
