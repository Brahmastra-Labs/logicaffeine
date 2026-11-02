import sys
n = int(sys.argv[1]); MOD = 1000000007
m = 1000; xs = list(range(m)); s = 0
for _ in range(n):
    for i, v in enumerate(xs, 1):
        s = (s + i + v) % MOD
print(s)
