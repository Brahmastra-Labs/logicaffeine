import sys

def gcd(a, b):
    x = a if a >= 0 else -a
    y = b if b >= 0 else -b
    while y > 0:
        x, y = y, x % y
    return x

def lcm(a, b):
    if a == 0:
        return 0
    if b == 0:
        return 0
    g = gcd(a, b)
    p = abs(a * b)
    return p // g

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += lcm(i, 60)
print(s)
