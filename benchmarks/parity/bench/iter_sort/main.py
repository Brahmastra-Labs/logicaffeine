import sys
reps = int(sys.argv[1])
M = 2048
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    res = sorted(xs)
    checksum += res[r % M]
print(checksum)
