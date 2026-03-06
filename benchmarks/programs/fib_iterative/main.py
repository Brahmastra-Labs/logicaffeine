import sys

n = int(sys.argv[1])
a, b = 0, 1
for _ in range(n):
    a, b = b, (a + b) % 1000000007
print(a)
