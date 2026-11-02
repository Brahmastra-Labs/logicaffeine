import sys
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    i = (r % 6000) + 1
    checksum += (xs[i-1] if 1 <= i <= M else -1)
print(checksum)
