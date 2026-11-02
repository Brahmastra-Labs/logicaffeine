import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int(math.dist((i/1000.0, i/700.0), (3.0, 4.0)) * 1000000)
print(s)
