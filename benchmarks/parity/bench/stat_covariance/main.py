import sys, math
n = int(sys.argv[1])
M = 4096
xs = [float(((k*2654435761)%999)+1) for k in range(M)]
ys = [float(((k*1013904223+12345)%997)+1) for k in range(M)]
checksum = 0
for r in range(n):
    xs[r % M] = float((r%999)+1)
    v = (lambda mx,my: sum((xs[i]-mx)*(ys[i]-my) for i in range(len(xs)))/(len(xs)-1))(sum(xs)/len(xs), sum(ys)/len(ys))
    checksum += int(v * 100)
print(checksum)
