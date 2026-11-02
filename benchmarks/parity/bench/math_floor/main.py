import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += math.floor(i / 7.0)
print(s)
