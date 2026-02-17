import sys
n = int(sys.argv[1])
s, sign = 0.0, 1.0
for k in range(n): s += sign / (2.0 * k + 1.0); sign = -sign
print(f"{s * 4.0:.15f}")
