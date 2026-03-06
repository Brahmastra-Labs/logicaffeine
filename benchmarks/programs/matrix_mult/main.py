import sys
MOD = 1000000007
n = int(sys.argv[1])
a = [0] * (n * n)
b = [0] * (n * n)
c = [0] * (n * n)
for i in range(n):
    for j in range(n):
        a[i * n + j] = (i * n + j) % 100
        b[i * n + j] = (j * n + i) % 100
for i in range(n):
    for k in range(n):
        for j in range(n):
            c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) % MOD
checksum = 0
for v in c:
    checksum = (checksum + v) % MOD
print(checksum)
