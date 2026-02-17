const std = @import("std");

const MAX_EDGES: usize = 5;

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));

    const allocator = std.heap.page_allocator;
    const adj = try allocator.alloc(usize, n * MAX_EDGES);
    defer allocator.free(adj);
    const adj_count = try allocator.alloc(usize, n);
    defer allocator.free(adj_count);
    @memset(adj_count, 0);

    const primes = [5]usize{ 31, 37, 41, 43, 47 };
    const offsets = [5]usize{ 7, 13, 17, 23, 29 };
    for (0..MAX_EDGES) |p| {
        for (0..n) |i| {
            const neighbor = (i * primes[p] + offsets[p]) % n;
            if (neighbor != i) {
                adj[i * MAX_EDGES + adj_count[i]] = neighbor;
                adj_count[i] += 1;
            }
        }
    }

    const queue = try allocator.alloc(usize, n);
    defer allocator.free(queue);
    const dist = try allocator.alloc(i64, n);
    defer allocator.free(dist);
    @memset(dist, -1);

    var front: usize = 0;
    var back: usize = 0;
    queue[back] = 0;
    back += 1;
    dist[0] = 0;
    while (front < back) {
        const v = queue[front];
        front += 1;
        for (0..adj_count[v]) |e| {
            const u = adj[v * MAX_EDGES + e];
            if (dist[u] == -1) {
                dist[u] = dist[v] + 1;
                queue[back] = u;
                back += 1;
            }
        }
    }
    var reachable: i64 = 0;
    var total_dist: i64 = 0;
    for (0..n) |i| {
        if (dist[i] >= 0) {
            reachable += 1;
            total_dist += dist[i];
        }
    }
    try stdout.interface.print("{d} {d}\n", .{ reachable, total_dist });
    try stdout.interface.flush();
}
