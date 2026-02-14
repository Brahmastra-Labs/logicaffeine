const std = @import("std");

fn fib(n: i64) i64 {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

pub fn main() !void {
    var args = std.process.args();
    _ = args.next();
    const arg = args.next() orelse return error.MissingArgument;
    const n = try std.fmt.parseInt(i64, arg, 10);
    var buf: [64]u8 = undefined;
    const s = try std.fmt.bufPrint(&buf, "{}\n", .{fib(n)});
    try std.fs.File.stdout().writeAll(s);
}
