import sys
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    pos = (r % M) + 1
    res = xs[:pos-1] + [r] + xs[pos-1:]
    checksum += res[pos-1]
print(checksum)
