import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += 1 + len(str(i)) + 2
print(s)
