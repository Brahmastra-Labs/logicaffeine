import sys
n = int(sys.argv[1])
M = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
s = 0
for i in range(n):
    m = 1 + (i % 12)
    s += len(M[m - 1])
print(s)
