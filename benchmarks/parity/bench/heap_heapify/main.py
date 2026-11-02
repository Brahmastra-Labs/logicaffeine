import sys
import heapq
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    h = list(xs)
    heapq.heapify(h)
    checksum += h[0]
print(checksum)
