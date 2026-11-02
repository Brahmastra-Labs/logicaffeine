import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += math.prod(range(i % 1000 + 1, i % 1000 + 5))
print(s)
