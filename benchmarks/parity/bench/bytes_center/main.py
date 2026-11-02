import sys
reps = int(sys.argv[1])
M = 4096
xs = bytearray((k * 2654435761) % 256 for k in range(M))
checksum = 0
for r in range(reps):
    xs[r % M] = r % 256
    res = xs.center(M + 100, b' ')
    checksum += res[r % M + 50]
print(checksum)
