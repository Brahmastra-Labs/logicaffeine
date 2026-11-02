import sys, math
n = int(sys.argv[1])
M = 4096
xs = [float(((k*2654435761)%999)+1) for k in range(M)]
checksum = 0
for r in range(n):
    xs[r % M] = float((r%999)+1)
    v = len(xs)/sum(1.0/x for x in xs)
    checksum += int(v * 1000)
print(checksum)
