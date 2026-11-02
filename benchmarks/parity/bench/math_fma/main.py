import sys
import math
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    s += int(math.fma(i/1000.0, i/700.0, 5.0) * 1000) if hasattr(math,'fma') else int((i/1000.0*(i/700.0)+5.0)*1000)
print(s)
