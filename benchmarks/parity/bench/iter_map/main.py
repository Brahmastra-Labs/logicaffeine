import sys
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    res = [(n * 3 + 1) % 1048576 for n in xs]
    checksum += res[r % M]
print(checksum)
