import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += math.perm(i % 20 + 5, 3)
print(s)
