import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += abs(i - n // 2)
print(s)
