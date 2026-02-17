const std = @import("std");
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const capacity = n * 5;
    const allocator = std.heap.page_allocator;
    var prev = try allocator.alloc(i64, capacity + 1);
    defer allocator.free(prev);
    var curr = try allocator.alloc(i64, capacity + 1);
    defer allocator.free(curr);
    @memset(prev, 0);
    for (0..n) |i| {
        const w = (i * 17 + 3) % 50 + 1;
        const v: i64 = @intCast((i * 31 + 7) % 100 + 1);
        for (0..capacity + 1) |j| {
            curr[j] = prev[j];
            if (j >= w and prev[j - w] + v > curr[j]) curr[j] = prev[j - w] + v;
        }
        const t = prev; prev = curr; curr = t;
    }
    try stdout.interface.print("{}\n", .{prev[capacity]});
    try stdout.interface.flush();
}
