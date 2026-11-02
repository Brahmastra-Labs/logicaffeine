import sys
import bisect
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
xs.sort()
checksum = 0
for r in range(reps):
    t = r % 1048576
    checksum += bisect.bisect_left(xs, t) + 1
print(checksum)
