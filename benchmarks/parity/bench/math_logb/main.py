import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int(math.floor(math.log2(abs(i/1000.0))) * 1)
print(s)
