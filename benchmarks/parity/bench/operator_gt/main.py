import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    if i > 1500000:
        s += 1
print(s)
