import sys
n = int(sys.argv[1])
M = ["January", "February", "March", "April", "May", "June",
     "July", "August", "September", "October", "November", "December"]
s = 0
for i in range(n):
    m = 1 + (i % 12)
    s += len(M[m - 1])
print(s)
