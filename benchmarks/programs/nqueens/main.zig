const std = @import("std");

fn solve(row: i64, cols: i64, diag1: i64, diag2: i64, n: i64) i64 {
    if (row == n) return 1;
    var count: i64 = 0;
    var available: i64 = ((@as(i64, 1) << @intCast(n)) - 1) & ~(cols | diag1 | diag2);
    while (available != 0) {
        const bit: i64 = available & (-available);
        available ^= bit;
        count += solve(row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1, n);
    }
    return count;
}

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: i64 = try std.fmt.parseInt(i64, n_str, 10);
    try stdout.interface.print("{}\n", .{solve(0, 0, 0, 0, n)});
    try stdout.interface.flush();
}
