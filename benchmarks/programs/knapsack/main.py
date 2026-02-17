import sys
n = int(sys.argv[1])
capacity = n * 5
prev = [0] * (capacity + 1)
for i in range(n):
    w = (i * 17 + 3) % 50 + 1
    v = (i * 31 + 7) % 100 + 1
    curr = prev[:]
    for j in range(w, capacity + 1):
        if prev[j - w] + v > curr[j]: curr[j] = prev[j - w] + v
    prev = curr
print(prev[capacity])
