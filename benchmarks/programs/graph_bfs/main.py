import sys
MAX_EDGES = 5
n = int(sys.argv[1])
primes = [31, 37, 41, 43, 47]
offsets = [7, 13, 17, 23, 29]
adj = [0] * (n * MAX_EDGES)
adj_count = [0] * n
for p in range(MAX_EDGES):
    for i in range(n):
        neighbor = (i * primes[p] + offsets[p]) % n
        if neighbor != i:
            adj[i * MAX_EDGES + adj_count[i]] = neighbor
            adj_count[i] += 1
queue = [0] * n
dist = [-1] * n
front, back = 0, 0
queue[back] = 0; back += 1
dist[0] = 0
while front < back:
    v = queue[front]; front += 1
    for e in range(adj_count[v]):
        u = adj[v * MAX_EDGES + e]
        if dist[u] == -1:
            dist[u] = dist[v] + 1
            queue[back] = u; back += 1
reachable = 0
total_dist = 0
for i in range(n):
    if dist[i] >= 0: reachable += 1; total_dist += dist[i]
print(reachable, total_dist)
