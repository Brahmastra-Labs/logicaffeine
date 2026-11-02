import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    c = i % 128
    if 32 <= c <= 126:
        s += 1
print(s)
