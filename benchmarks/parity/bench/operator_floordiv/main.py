import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += (i // 7)
print(s)
