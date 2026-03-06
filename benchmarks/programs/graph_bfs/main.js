const MAX_EDGES = 5;
const n = parseInt(process.argv[2]);
const primes = [31, 37, 41, 43, 47];
const offsets = [7, 13, 17, 23, 29];
const adj = new Int32Array(n * MAX_EDGES);
const adjCount = new Int32Array(n);
for (let p = 0; p < MAX_EDGES; p++) {
    for (let i = 0; i < n; i++) {
        const neighbor = (i * primes[p] + offsets[p]) % n;
        if (neighbor !== i) {
            adj[i * MAX_EDGES + adjCount[i]] = neighbor;
            adjCount[i]++;
        }
    }
}
const queue = new Int32Array(n);
const dist = new Int32Array(n).fill(-1);
let front = 0, back = 0;
queue[back++] = 0;
dist[0] = 0;
while (front < back) {
    const v = queue[front++];
    for (let e = 0; e < adjCount[v]; e++) {
        const u = adj[v * MAX_EDGES + e];
        if (dist[u] === -1) { dist[u] = dist[v] + 1; queue[back++] = u; }
    }
}
let reachable = 0, totalDist = 0;
for (let i = 0; i < n; i++) {
    if (dist[i] >= 0) { reachable++; totalDist += dist[i]; }
}
console.log(reachable + " " + totalDist);
