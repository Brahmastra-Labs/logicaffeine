import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    c = i % 128
    s += ((c - 48) if 48 <= c <= 57 else -1)
print(s)
