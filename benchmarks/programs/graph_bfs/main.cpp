#include <cstdio>
#include <cstdlib>
#include <cstring>

#define MAX_EDGES 5

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long primes[] = {31, 37, 41, 43, 47};
    long offsets[] = {7, 13, 17, 23, 29};
    long *adj = new long[n * MAX_EDGES];
    int *adj_count = new int[n]();
    for (int p = 0; p < MAX_EDGES; p++) {
        for (long i = 0; i < n; i++) {
            long neighbor = (i * primes[p] + offsets[p]) % n;
            if (neighbor != i) {
                adj[i * MAX_EDGES + adj_count[i]] = neighbor;
                adj_count[i]++;
            }
        }
    }
    long *queue = new long[n];
    long *dist = new long[n];
    memset(dist, -1, n * sizeof(long));
    long front = 0, back = 0;
    queue[back++] = 0;
    dist[0] = 0;
    while (front < back) {
        long v = queue[front++];
        for (int e = 0; e < adj_count[v]; e++) {
            long u = adj[v * MAX_EDGES + e];
            if (dist[u] == -1) { dist[u] = dist[v] + 1; queue[back++] = u; }
        }
    }
    long reachable = 0, total_dist = 0;
    for (long i = 0; i < n; i++) {
        if (dist[i] >= 0) { reachable++; total_dist += dist[i]; }
    }
    printf("%ld %ld\n", reachable, total_dist);
    delete[] adj; delete[] adj_count; delete[] queue; delete[] dist;
    return 0;
}
