import sys
import bisect
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
xs.sort()
checksum = 0
for r in range(reps):
    t = r % 1048576
    i = bisect.bisect_left(xs, t)
    checksum += (i + 1 if i < M and xs[i] == t else 0)
print(checksum)
