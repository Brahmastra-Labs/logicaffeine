import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int('{:032b}'.format(i & 0xFFFFFFFF)[::-1], 2)
print(s)
