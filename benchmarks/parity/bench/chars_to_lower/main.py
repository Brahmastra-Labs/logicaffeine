import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    c = i % 128
    s += ((c + 32) if 65 <= c <= 90 else c)
print(s)
