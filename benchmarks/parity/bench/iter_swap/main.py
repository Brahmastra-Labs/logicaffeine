import sys
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    res = xs[:]
    res[0], res[r % M] = res[r % M], res[0]
    checksum += res[0]
print(checksum)
