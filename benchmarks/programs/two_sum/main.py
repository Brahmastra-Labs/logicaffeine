import sys
n = int(sys.argv[1])
arr = []
seed = 42
for _ in range(n): seed=(seed*1103515245+12345)%2147483648; arr.append(((seed>>16)&0x7fff)%n)
seen = set(); count = 0
for x in arr:
    c = n - x
    if c >= 0 and c in seen: count += 1
    seen.add(x)
print(count)
