import sys
def powexact(base, exp):
    acc=1; b=base; e=exp
    while e>0:
        if e%2==1: acc=acc*b
        e//=2; b=b*b
    return acc
n=int(sys.argv[1]); s=0
for i in range(1,n+1): s+=powexact((i%5)+1, i%13)
print(s)
