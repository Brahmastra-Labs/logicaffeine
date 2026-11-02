import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int((lambda v: math.floor(v+0.5) if (v-math.floor(v)!=0.5) else 2*round(v/2))(i/2000.0) * 1000000)
print(s)
