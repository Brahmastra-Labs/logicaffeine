const std = @import("std");

fn solve(row: i32, cols: i32, diag1: i32, diag2: i32, n: i32) i32 {
    if (row == n) return 1;
    var count: i32 = 0;
    var available: i32 = ((@as(i32, 1) << @intCast(n)) - 1) & ~(cols | diag1 | diag2);
    while (available != 0) {
        const bit: i32 = available & (-available);
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
    const n: i32 = try std.fmt.parseInt(i32, n_str, 10);
    try stdout.interface.print("{}\n", .{solve(0, 0, 0, 0, n)});
    try stdout.interface.flush();
}
