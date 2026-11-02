import sys
import itertools
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    checksum += sum(1 for _ in itertools.takewhile(lambda n: n < 2000000, xs))
print(checksum)
