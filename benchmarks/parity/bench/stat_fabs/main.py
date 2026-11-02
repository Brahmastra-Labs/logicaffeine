import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int(abs(i - 2500000.0) * 100)
print(s)
