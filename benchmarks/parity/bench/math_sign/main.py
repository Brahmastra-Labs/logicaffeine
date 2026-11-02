import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += ((i - 10000000 > 0) - (i - 10000000 < 0))
print(s)
