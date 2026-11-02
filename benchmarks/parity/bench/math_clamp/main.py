import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += min(15000000, max(5000000, i))
print(s)
