import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += 1 if math.isinf(i) else 0
print(s)
