const MAX_EDGES = 5;
const n = parseInt(process.argv[2]);
const primes = [31, 37, 41, 43, 47];
const offsets = [7, 13, 17, 23, 29];
const adj = [];
for (let k = 0; k < n * MAX_EDGES; k++) adj.push(0);
const adjCount = [];
for (let i = 0; i < n; i++) adjCount.push(0);
for (let p = 0; p < MAX_EDGES; p++) {
    for (let i = 0; i < n; i++) {
        const neighbor = (i * primes[p] + offsets[p]) % n;
        if (neighbor !== i) {
            adj[i * MAX_EDGES + adjCount[i]] = neighbor;
            adjCount[i]++;
        }
    }
}
const queue = [];
for (let i = 0; i < n; i++) queue.push(0);
const dist = [];
for (let i = 0; i < n; i++) dist.push(-1);
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
let h = 0;
for (let i = 0; i < n; i++) h = (h * 31 + dist[i] + 1) & 268435455;
console.log(reachable + " " + totalDist + " " + h);
