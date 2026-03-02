const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    var count: i64 = 0;
    var i: i64 = 2;
    while (i <= n) : (i += 1) {
        var is_prime = true;
        var d: i64 = 2;
        while (d * d <= i) : (d += 1) {
            if (@mod(i, d) == 0) { is_prime = false; break; }
        }
        if (is_prime) count += 1;
    }
    try stdout.interface.print("{}\n", .{count});
    try stdout.interface.flush();
}
