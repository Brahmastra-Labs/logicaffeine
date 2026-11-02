const std = @import("std");
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const capacity = n * 5;
    const allocator = std.heap.smp_allocator;
    const weight = try allocator.alloc(i64, n);
    defer allocator.free(weight);
    const value = try allocator.alloc(i64, n);
    defer allocator.free(value);
    var seed: i64 = 42;
    for (0..n) |i| {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        weight[i] = @rem(seed >> 16, 50) + 1;
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        value[i] = @rem(seed >> 16, 100) + 1;
    }
    var prev = try allocator.alloc(i64, capacity + 1);
    defer allocator.free(prev);
    var curr = try allocator.alloc(i64, capacity + 1);
    defer allocator.free(curr);
    @memset(prev, 0);
    for (0..n) |i| {
        const w: usize = @intCast(weight[i]);
        const v = value[i];
        for (0..capacity + 1) |j| {
            curr[j] = prev[j];
            if (j >= w and prev[j - w] + v > curr[j]) curr[j] = prev[j - w] + v;
        }
        const t = prev; prev = curr; curr = t;
    }
    var h: i64 = 0;
    for (0..capacity + 1) |j| {
        h = (h * 31 + prev[j]) & 268435455;
    }
    try stdout.interface.print("{} {}\n", .{ prev[capacity], h });
    try stdout.interface.flush();
}
