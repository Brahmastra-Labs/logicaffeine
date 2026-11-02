import sys
def isp(x):
    if x < 2:
        return False
    d = 2
    while d * d <= x:
        if x % d == 0:
            return False
        d += 1
    return True
n = int(sys.argv[1])
s = 0
for i in range(1, n + 1):
    if isp(i):
        s += 1
print(s)
