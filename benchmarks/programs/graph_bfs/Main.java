public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        int MAX_EDGES = 5;
        long[] primes = {31, 37, 41, 43, 47};
        long[] offsets = {7, 13, 17, 23, 29};
        long[] adj = new long[n * MAX_EDGES];
        long[] adjCount = new long[n];
        for (int p = 0; p < MAX_EDGES; p++) {
            for (int i = 0; i < n; i++) {
                long neighbor = (i * primes[p] + offsets[p]) % n;
                if (neighbor != i) {
                    adj[i * MAX_EDGES + (int) adjCount[i]] = neighbor;
                    adjCount[i]++;
                }
            }
        }
        long[] queue = new long[n];
        long[] dist = new long[n];
        java.util.Arrays.fill(dist, -1);
        int front = 0, back = 0;
        queue[back++] = 0;
        dist[0] = 0;
        while (front < back) {
            int v = (int) queue[front++];
            for (int e = 0; e < adjCount[v]; e++) {
                int u = (int) adj[v * MAX_EDGES + e];
                if (dist[u] == -1) { dist[u] = dist[v] + 1; queue[back++] = u; }
            }
        }
        long reachable = 0, totalDist = 0;
        for (int i = 0; i < n; i++) {
            if (dist[i] >= 0) { reachable++; totalDist += dist[i]; }
        }
        long h = 0;
        for (int i = 0; i < n; i++) h = (h * 31 + dist[i] + 1) & 268435455;
        System.out.println(reachable + " " + totalDist + " " + h);
    }
}
