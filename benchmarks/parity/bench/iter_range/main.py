import sys
n = int(sys.argv[1]); MOD = 1000000007; hi = 1000; s = 0
for _ in range(n):
    s = (s + len(list(range(0, hi + 1)))) % MOD
print(s)
