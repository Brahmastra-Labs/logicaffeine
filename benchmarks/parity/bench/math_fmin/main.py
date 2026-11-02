import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int(min(i/1000.0, i/1001.0) * 1000000)
print(s)
