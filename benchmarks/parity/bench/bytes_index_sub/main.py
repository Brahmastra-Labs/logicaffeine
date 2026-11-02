import sys
reps = int(sys.argv[1])
M = 4096
xs = bytearray((k * 2654435761) % 256 for k in range(M))
sub = bytes([72, 101, 108])
checksum = 0
for r in range(reps):
    xs[r % M] = r % 256
    idx = bytes(xs).find(sub)
    checksum += (idx + 1 if idx >= 0 else 0)
print(checksum)
