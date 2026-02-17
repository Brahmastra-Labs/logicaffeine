const std = @import("std");
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    var sum: f64 = 0;
    var sign: f64 = 1;
    var k: i64 = 0;
    while (k < n) : (k += 1) {
        sum += sign / (2.0 * @as(f64, @floatFromInt(k)) + 1.0);
        sign = -sign;
    }
    try stdout.interface.print("{d:.15}\n", .{sum * 4.0});
    try stdout.interface.flush();
}
