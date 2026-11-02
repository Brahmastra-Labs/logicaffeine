import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    c = i % 128
    if c in (9,10,11,12,13,32):
        s += 1
print(s)
