import sys
reps = int(sys.argv[1])
M = 4096
xs = bytearray((k * 2654435761) % 256 for k in range(M))
checksum = 0
for r in range(reps):
    xs[r % M] = r % 256
    t = r % 256
    checksum += (xs.index(t) + 1 if t in xs else 0)
print(checksum)
