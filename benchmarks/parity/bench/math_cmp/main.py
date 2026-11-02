import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += ((i > n - i) - (i < n - i))
print(s)
