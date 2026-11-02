import sys
n = int(sys.argv[1]); MOD = 1000000007
m = 1000; xs = list(range(m)); ys = [k + m for k in range(m)]; s = 0
for _ in range(n):
    for a, b in zip(xs, ys):
        s = (s + a + b) % MOD
print(s)
