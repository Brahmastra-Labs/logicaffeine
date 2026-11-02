import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int(10.0**(i%10) * 1000)
print(s)
