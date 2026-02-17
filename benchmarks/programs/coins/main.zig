const std = @import("std");
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const allocator = std.heap.page_allocator;
    var dp = try allocator.alloc(i64, n + 1);
    defer allocator.free(dp);
    @memset(dp, 0);
    dp[0] = 1;
    const coins = [_]usize{ 1, 5, 10, 25, 50, 100 };
    for (coins) |c| {
        var j = c;
        while (j <= n) : (j += 1) dp[j] = @mod(dp[j] + dp[j - c], 1000000007);
    }
    try stdout.interface.print("{}\n", .{dp[n]});
    try stdout.interface.flush();
}
