import sys, math
n=int(sys.argv[1])
M=4096
xs=[float(((k*2654435761)%999)+1) for k in range(M)]
checksum=0
for r in range(n):
 xs[r%M]=float((r%999)+1)
 checksum+=int(math.exp(sum(math.log(v) for v in xs)/len(xs))*1000)
print(checksum)
