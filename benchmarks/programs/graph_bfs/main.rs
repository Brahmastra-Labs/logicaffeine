use std::env;

const MAX_EDGES: usize = 5;

fn main() {
    let n: usize = env::args().nth(1).unwrap().parse().unwrap();
    let primes: [usize; 5] = [31, 37, 41, 43, 47];
    let offsets: [usize; 5] = [7, 13, 17, 23, 29];
    let mut adj = vec![0usize; n * MAX_EDGES];
    let mut adj_count = vec![0usize; n];
    for p in 0..MAX_EDGES {
        for i in 0..n {
            let neighbor = (i * primes[p] + offsets[p]) % n;
            if neighbor != i {
                adj[i * MAX_EDGES + adj_count[i]] = neighbor;
                adj_count[i] += 1;
            }
        }
    }
    let mut queue = vec![0usize; n];
    let mut dist = vec![-1i64; n];
    let mut front = 0usize;
    let mut back = 0usize;
    queue[back] = 0; back += 1;
    dist[0] = 0;
    while front < back {
        let v = queue[front]; front += 1;
        for e in 0..adj_count[v] {
            let u = adj[v * MAX_EDGES + e];
            if dist[u] == -1 { dist[u] = dist[v] + 1; queue[back] = u; back += 1; }
        }
    }
    let mut reachable: i64 = 0;
    let mut total_dist: i64 = 0;
    for i in 0..n {
        if dist[i] >= 0 { reachable += 1; total_dist += dist[i]; }
    }
    println!("{} {}", reachable, total_dist);
}
