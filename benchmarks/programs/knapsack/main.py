import sys
n = int(sys.argv[1])
capacity = n * 5
prev = [0] * (capacity + 1)
curr = [0] * (capacity + 1)
for i in range(n):
    w = (i * 17 + 3) % 50 + 1
    v = (i * 31 + 7) % 100 + 1
    for j in range(capacity + 1):
        curr[j] = prev[j]
        if j >= w and prev[j - w] + v > curr[j]:
            curr[j] = prev[j - w] + v
    prev, curr = curr, prev
print(prev[capacity])
