import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += ((((i & 0xFFFFFFFF) >> 7) | (i << 25)) & 0xFFFFFFFF)
print(s)
