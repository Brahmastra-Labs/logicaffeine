import sys
n = int(sys.argv[1])
t = 0
for i in range(1, n + 1):
    r = "hello world"[::-1]
    t += len(r)
print(t)
