import sys

n = int(sys.argv[1])
d = {}
for i in range(n):
    d[i] = i * 2
found = 0
for i in range(n):
    if d[i] == i * 2:
        found += 1
print(found)
