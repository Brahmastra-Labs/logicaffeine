import sys
n = int(sys.argv[1])
arr = [(i * 7 + 3) % 1000000 for i in range(n)]
s = 0
for v in arr:
    s = (s + v) % 1000000007
print(s)
