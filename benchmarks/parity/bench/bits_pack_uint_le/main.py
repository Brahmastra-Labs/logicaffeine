import sys
n = int(sys.argv[1]); MOD = 1000000007; s = 0
for r in range(n):
    bs = r.to_bytes(6, 'little')
    acc = 0
    for j in range(1, 7):
        acc += j * bs[j - 1]
    s = (s + acc) % MOD
print(s)
