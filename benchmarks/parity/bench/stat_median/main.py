import sys, math
n = int(sys.argv[1])
M = 4096
xs = [float(((k*2654435761)%999)+1) for k in range(M)]
checksum = 0
for r in range(n):
    xs[r % M] = float((r%999)+1)
    v = (lambda s: s[len(s)//2] if len(s)%2 else (s[len(s)//2-1]+s[len(s)//2])/2)(sorted(xs))
    checksum += int(v * 100)
print(checksum)
