import sys

def powmod(base, exp, m):
    acc = 1
    b = base % m
    e = exp
    while e > 0:
        if e % 2 == 1:
            acc = (acc * b) % m
        e //= 2
        b = (b * b) % m
    return acc

n = int(sys.argv[1])
m = 1000000007
s = 0
for i in range(1, n + 1):
    s += powmod(i, i, m)
print(s)
