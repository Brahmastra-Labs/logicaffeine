import sys
reps = int(sys.argv[1])
M = 4096
xs = bytearray((k * 2654435761) % 256 for k in range(M))
ys = bytearray((k * 1013904223 + 7) % 256 for k in range(M))
checksum = 0
for r in range(reps):
    xs[r % M] = r % 256
    checksum += (xs > ys) - (xs < ys)
print(checksum)
