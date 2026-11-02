import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    x = i - n // 2
    s += ((abs(x) > 1000000) - (abs(x) < 1000000))
print(s)
