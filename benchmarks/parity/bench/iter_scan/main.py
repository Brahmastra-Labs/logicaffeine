import sys
reps = int(sys.argv[1])
M = 4096
xs = [(k * 2654435761) % 1048576 for k in range(M)]
checksum = 0
for r in range(reps):
    xs[r % M] = r % 1048576
    acc = 0
    res = []
    for v in xs:
        acc += v
        res.append(acc)
    checksum += res[r % M]
print(checksum)
