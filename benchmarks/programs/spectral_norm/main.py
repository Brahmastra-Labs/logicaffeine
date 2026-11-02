import sys, math

def A(i,j): return 1.0/((i+j)*(i+j+1)//2+i+1)

def mulAv(n, v, out):
    for i in range(n):
        s = 0.0
        for j in range(n):
            s += A(i,j) * v[j]
        out[i] = s

def mulAtv(n, v, out):
    for i in range(n):
        s = 0.0
        for j in range(n):
            s += A(j,i) * v[j]
        out[i] = s

def mulAtAv(n, v, out, tmp):
    mulAv(n, v, tmp)
    mulAtv(n, tmp, out)

def main():
    n = int(sys.argv[1])
    u = [1.0] * n
    v = [0.0] * n
    tmp = [0.0] * n
    for _ in range(10):
        mulAtAv(n, u, v, tmp)
        mulAtAv(n, v, u, tmp)
    vBv = vv = 0.0
    for i in range(n):
        vBv += u[i] * v[i]
        vv += v[i] * v[i]
    print(f"{math.sqrt(vBv/vv):.9f}")

if __name__ == "__main__":
    main()
