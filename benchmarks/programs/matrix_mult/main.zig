const std = @import("std");

const MOD: i64 = 1000000007;

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));

    const allocator = std.heap.page_allocator;
    const a = try allocator.alloc(i64, n * n);
    defer allocator.free(a);
    const b = try allocator.alloc(i64, n * n);
    defer allocator.free(b);
    const c = try allocator.alloc(i64, n * n);
    defer allocator.free(c);
    @memset(c, 0);

    for (0..n) |i| {
        for (0..n) |j| {
            a[i * n + j] = @intCast((i * n + j) % 100);
            b[i * n + j] = @intCast((j * n + i) % 100);
        }
    }
    for (0..n) |i| {
        for (0..n) |k| {
            for (0..n) |j| {
                c[i * n + j] = @mod(c[i * n + j] + a[i * n + k] * b[k * n + j], MOD);
            }
        }
    }
    var checksum: i64 = 0;
    for (c) |v| { checksum = @mod(checksum + v, MOD); }
    try stdout.interface.print("{d}\n", .{checksum});
    try stdout.interface.flush();
}
