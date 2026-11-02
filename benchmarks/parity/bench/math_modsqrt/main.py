import sys

n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    im = i % 1000003
    s += min(im, 1000003 - im)
print(s)
