const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    var total: i64 = 0;
    var i: i64 = 1;
    while (i <= n) : (i += 1) {
        var k: i64 = i;
        while (k != 1) {
            if (@mod(k, 2) == 0) {
                k = @divTrunc(k, 2);
            } else {
                k = 3 * k + 1;
            }
            total += 1;
        }
    }
    try stdout.interface.print("{}\n", .{total});
    try stdout.interface.flush();
}
