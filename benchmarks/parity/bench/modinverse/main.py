import sys
def modinverse(x, m):
    oldR=x%m
    if oldR<0: oldR+=m
    r=m; oldS=1; s=0
    while r>0:
        q=oldR//r
        oldR,r = r, oldR-q*r
        oldS,s = s, oldS-q*s
    if oldR>1: return 0
    inv=oldS%m
    if inv<0: return inv+m
    return inv
n=int(sys.argv[1]); m=1000000007; s=0
for i in range(1,n+1): s+=modinverse(i,m)
print(s)
