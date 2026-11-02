import sys, math
n=int(sys.argv[1])
M=4096
xs=[float(((k*2654435761)%999)+1) for k in range(M)]
ys=[float(((k*1013904223+12345)%997)+1) for k in range(M)]
checksum=0
for r in range(n):
 xs[r%M]=float((r%999)+1)
 mx=sum(xs)/len(xs); my=sum(ys)/len(ys)
 L=len(xs)
 cov=sum((xs[i]-mx)*(ys[i]-my) for i in range(L))/(L-1)
 var=sum((x-mx)**2 for x in xs)/(L-1)
 slope=cov/var
 intercept=my-slope*mx
 checksum+=int(slope*1000000)+int(intercept*100)
print(checksum)
