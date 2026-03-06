import sys, math
def A(i,j): return 1.0/((i+j)*(i+j+1)//2+i+1)
def mulAv(n,v): return [sum(A(i,j)*v[j] for j in range(n)) for i in range(n)]
def mulAtv(n,v): return [sum(A(j,i)*v[j] for j in range(n)) for i in range(n)]
def mulAtAv(n,v): return mulAtv(n,mulAv(n,v))
n=int(sys.argv[1])
u=[1.0]*n
for _ in range(10): v=mulAtAv(n,u); u=mulAtAv(n,v)
vBv=sum(u[i]*v[i] for i in range(n))
vv=sum(v[i]*v[i] for i in range(n))
print(f"{math.sqrt(vBv/vv):.9f}")
