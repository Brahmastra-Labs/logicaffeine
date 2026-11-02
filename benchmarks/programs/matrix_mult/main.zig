const std = @import("std");

const MOD: i64 = 1000000007;

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));

    const allocator = std.heap.smp_allocator;
    const a = try allocator.alloc(i64, n * n);
    defer allocator.free(a);
    const b = try allocator.alloc(i64, n * n);
    defer allocator.free(b);
    const c = try allocator.alloc(i64, n * n);
    defer allocator.free(c);
    @memset(c, 0);

    var seed: i64 = 42;
    for (0..n * n) |i| {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        a[i] = @rem(@divTrunc(seed, 65536), 100);
    }
    for (0..n * n) |i| {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        b[i] = @rem(@divTrunc(seed, 65536), 100);
    }
    for (0..n) |i| {
        for (0..n) |k| {
            for (0..n) |j| {
                c[i * n + j] = @rem(c[i * n + j] + a[i * n + k] * b[k * n + j], MOD);
            }
        }
    }
    var checksum: i64 = 0;
    for (c) |v| { checksum = (checksum * 31 + v) & 268435455; }
    try stdout.interface.print("{d}\n", .{checksum});
    try stdout.interface.flush();
}
