import sys, math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += math.factorial((i % 12) + 1)
print(s)
