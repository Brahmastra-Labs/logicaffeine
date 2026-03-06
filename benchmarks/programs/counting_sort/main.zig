const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const allocator = std.heap.page_allocator;
    var arr = try allocator.alloc(i64, n);
    defer allocator.free(arr);
    var seed: i64 = 42;
    for (0..n) |i| {
        seed = @mod(seed *% 1103515245 + 12345, 2147483648);
        arr[i] = @mod(@divTrunc(seed, 65536), 1000);
    }
    var counts = [_]i64{0} ** 1000;
    for (arr) |v| { counts[@intCast(v)] += 1; }
    var sorted = try allocator.alloc(i64, n);
    defer allocator.free(sorted);
    var idx: usize = 0;
    for (0..1000) |v| {
        var c: i64 = 0;
        while (c < counts[v]) : (c += 1) {
            sorted[idx] = @intCast(v);
            idx += 1;
        }
    }
    var checksum: i64 = 0;
    for (sorted) |v| { checksum = @mod(checksum + v, 1000000007); }
    try stdout.interface.print("{} {} {}\n", .{ sorted[0], sorted[n - 1], checksum });
    try stdout.interface.flush();
}
