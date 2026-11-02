import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int(math.cbrt(i) * 1000)
print(s)
