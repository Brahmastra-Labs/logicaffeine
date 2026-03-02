public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        int MAX_EDGES = 5;
        int[] primes = {31, 37, 41, 43, 47};
        int[] offsets = {7, 13, 17, 23, 29};
        int[] adj = new int[n * MAX_EDGES];
        int[] adjCount = new int[n];
        for (int p = 0; p < MAX_EDGES; p++) {
            for (int i = 0; i < n; i++) {
                int neighbor = (int)(((long)i * primes[p] + offsets[p]) % n);
                if (neighbor != i) {
                    adj[i * MAX_EDGES + adjCount[i]] = neighbor;
                    adjCount[i]++;
                }
            }
        }
        int[] queue = new int[n];
        long[] dist = new long[n];
        java.util.Arrays.fill(dist, -1);
        int front = 0, back = 0;
        queue[back++] = 0;
        dist[0] = 0;
        while (front < back) {
            int v = queue[front++];
            for (int e = 0; e < adjCount[v]; e++) {
                int u = adj[v * MAX_EDGES + e];
                if (dist[u] == -1) { dist[u] = dist[v] + 1; queue[back++] = u; }
            }
        }
        long reachable = 0, totalDist = 0;
        for (int i = 0; i < n; i++) {
            if (dist[i] >= 0) { reachable++; totalDist += dist[i]; }
        }
        System.out.println(reachable + " " + totalDist);
    }
}
