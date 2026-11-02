import sys
n = int(sys.argv[1])
D = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]
s = 0
for i in range(n):
    s += len(D[i % 7])
print(s)
