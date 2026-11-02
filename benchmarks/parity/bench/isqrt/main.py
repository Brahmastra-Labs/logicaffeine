import sys

def isqrt(n):
    if n < 2:
        return n
    x = n
    y = (x + 1) // 2
    while y < x:
        x = y
        y = (x + n // x) // 2
    return x

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += isqrt(i)
print(s)
