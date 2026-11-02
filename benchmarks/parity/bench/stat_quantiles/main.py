import sys, math
n=int(sys.argv[1])
M=4096
xs=[float(((k*2654435761)%999)+1) for k in range(M)]
def quant(a,i,nn):
 s=sorted(a); ld=len(s); mm=ld+1
 j=(i*mm)//nn; delta=(i*mm)%nn
 j=max(1,min(j,ld-1))
 lo=s[j-1]; hi=s[j]
 return (lo*(nn-delta)+hi*delta)/nn
checksum=0
for r in range(n):
 xs[r%M]=float((r%999)+1)
 checksum+=int(quant(xs,(r%3)+1,4)*100)
print(checksum)
