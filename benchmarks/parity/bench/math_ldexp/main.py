import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int(math.ldexp(i/1000000.0, i%10) * 1000000)
print(s)
