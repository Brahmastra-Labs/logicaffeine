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
    for (0..n) |i| {
        arr[i] = @mod(@as(i64, @intCast(i)) * 7 + 3, 1000000);
    }
    var sum: i64 = 0;
    for (arr) |v| {
        sum = @mod(sum + v, 1000000007);
    }
    try stdout.interface.print("{}\n", .{sum});
    try stdout.interface.flush();
}
