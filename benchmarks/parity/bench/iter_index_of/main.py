import sys
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    t = r % 2097152
    checksum += (xs.index(t)+1 if t in xs else 0)
print(checksum)
