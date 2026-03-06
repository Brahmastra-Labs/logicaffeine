const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    var sum: i64 = 0;
    var i: i64 = 1;
    while (i <= n) : (i += 1) {
        sum = @mod(sum + i, 1000000007);
    }
    try stdout.interface.print("{}\n", .{sum});
    try stdout.interface.flush();
}
