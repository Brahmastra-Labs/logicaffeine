const std = @import("std");

fn ackermann(m: i64, n: i64) i64 {
    if (m == 0) return n + 1;
    if (n == 0) return ackermann(m - 1, 1);
    return ackermann(m - 1, ackermann(m, n - 1));
}

pub fn main() !void {
    const stdout = std.io.getStdOut().writer();
    var args = std.process.args();
    _ = args.skip();
    const arg = args.next() orelse return;
    const m = try std.fmt.parseInt(i64, arg, 10);
    try stdout.print("{}\n", .{ackermann(3, m)});
}
