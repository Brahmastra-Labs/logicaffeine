import sys
def jacobi(a, n):
    a %= n
    r = 1
    while a:
        while a % 2 == 0:
            a //= 2
            if n % 8 in (3, 5):
                r = -r
        a, n = n, a
        if a % 4 == 3 and n % 4 == 3:
            r = -r
        a %= n
    return r if n == 1 else 0
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += jacobi(i, 2 * i + 1)
print(s)
