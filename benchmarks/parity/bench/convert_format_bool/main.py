import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += (4 if i % 2 == 0 else 5)
print(s)
