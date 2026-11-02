import sys, math
n=int(sys.argv[1])
M=4096
xs=[float(((k*2654435761)%999)+1) for k in range(M)]
def fs(a):
 s=0.0;c=0.0
 for v in a:
  t=s+v
  c += (s-t)+v if abs(s)>=abs(v) else (v-t)+s
  s=t
 return s+c
checksum=0
for r in range(n):
 xs[r%M]=float((r%999)+1)
 checksum+=int(fs(xs)*100)
print(checksum)
