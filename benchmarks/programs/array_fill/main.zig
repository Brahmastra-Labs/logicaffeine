const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const allocator = std.heap.smp_allocator;
    var arr = try allocator.alloc(i64, n);
    defer allocator.free(arr);
    for (0..n) |i| {
        arr[i] = @rem(@as(i64, @intCast(i)) * 7 + 3, 1000000);
    }
    var h: i64 = 0;
    for (arr) |v| {
        h = (h * 31 + v) & 268435455;
    }
    try stdout.interface.print("{}\n", .{h});
    try stdout.interface.flush();
}
