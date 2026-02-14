const std = @import("std");

fn ackermann(m: i64, n: i64) i64 {
    if (m == 0) return n + 1;
    if (n == 0) return ackermann(m - 1, 1);
    return ackermann(m - 1, ackermann(m, n - 1));
}

pub fn main() !void {
    var args = std.process.args();
    _ = args.next();
    const arg = args.next() orelse return error.MissingArgument;
    const m = try std.fmt.parseInt(i64, arg, 10);
    var buf: [64]u8 = undefined;
    const s = try std.fmt.bufPrint(&buf, "{}\n", .{ackermann(3, m)});
    try std.fs.File.stdout().writeAll(s);
}
