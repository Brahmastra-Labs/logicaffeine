#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_EDGES 5

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long primes[] = {31, 37, 41, 43, 47};
    long offsets[] = {7, 13, 17, 23, 29};

    long *adj = malloc(n * MAX_EDGES * sizeof(long));
    int *adj_count = calloc(n, sizeof(int));
    for (int p = 0; p < MAX_EDGES; p++) {
        for (long i = 0; i < n; i++) {
            long neighbor = (i * primes[p] + offsets[p]) % n;
            if (neighbor != i) {
                int idx = i * MAX_EDGES + adj_count[i];
                adj[idx] = neighbor;
                adj_count[i]++;
            }
        }
    }

    long *queue = malloc(n * sizeof(long));
    long *dist = malloc(n * sizeof(long));
    memset(dist, -1, n * sizeof(long));
    long front = 0, back = 0;
    queue[back++] = 0;
    dist[0] = 0;
    while (front < back) {
        long v = queue[front++];
        for (int e = 0; e < adj_count[v]; e++) {
            long u = adj[v * MAX_EDGES + e];
            if (dist[u] == -1) {
                dist[u] = dist[v] + 1;
                queue[back++] = u;
            }
        }
    }
    long reachable = 0, total_dist = 0;
    for (long i = 0; i < n; i++) {
        if (dist[i] >= 0) { reachable++; total_dist += dist[i]; }
    }
    printf("%ld %ld\n", reachable, total_dist);
    free(adj); free(adj_count); free(queue); free(dist);
    return 0;
}
