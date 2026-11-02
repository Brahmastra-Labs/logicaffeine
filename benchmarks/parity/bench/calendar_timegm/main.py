import sys, calendar
n = int(sys.argv[1])
MOD = 1000000007
s = 0
for i in range(n):
    y = 2000 + (i % 50)
    mo = 1 + (i % 12)
    d = 1 + (i % 28)
    h = i % 24
    mi = i % 60
    se = i % 60
    s = (s + calendar.timegm((y, mo, d, h, mi, se, 0, 0, 0))) % MOD
print(s)
