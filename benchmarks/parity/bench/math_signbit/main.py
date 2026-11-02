import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += 1 if math.copysign(1.0,(i%2)*2-1) < 0 else 0
print(s)
