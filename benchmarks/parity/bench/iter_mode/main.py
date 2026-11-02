import sys, statistics
n = int(sys.argv[1]); MOD = 1000000007
xs = list(range(1, 301)) + [5, 5, 5]; s = 0
for _ in range(n):
    s = (s + statistics.mode(xs)) % MOD
print(s)
