import sys, bisect
n = int(sys.argv[1]); MOD = 1000000007
m = 1000; xs = list(range(m)); s = 0
for r in range(n):
    c = xs[:]; bisect.insort(c, r % (m + 1)); s = (s + len(c)) % MOD
print(s)
