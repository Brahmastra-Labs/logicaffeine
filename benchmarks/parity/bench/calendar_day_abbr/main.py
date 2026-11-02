import sys
n = int(sys.argv[1])
D = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"]
s = 0
for i in range(n):
    s += len(D[i % 7])
print(s)
