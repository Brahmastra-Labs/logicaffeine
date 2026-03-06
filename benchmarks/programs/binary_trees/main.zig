const std = @import("std");
fn makeCheck(d: i32) i64 { if (d == 0) return 1; return 1 + makeCheck(d-1) + makeCheck(d-1); }
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i32, n_str, 10);
    const mn: i32 = 4; const mx = if (mn + 2 > n) mn + 2 else n;
    try stdout.interface.print("stretch tree of depth {}\t check: {}\n", .{mx+1, makeCheck(mx+1)});
    const ll = makeCheck(mx);
    var d = mn;
    while (d <= mx) : (d += 2) {
        const shift: u5 = @intCast(mx - d + mn);
        const it: i32 = @as(i32, 1) << shift;
        var tc: i64 = 0;
        var i: i32 = 0;
        while (i < it) : (i += 1) tc += makeCheck(d);
        try stdout.interface.print("{}\t trees of depth {}\t check: {}\n", .{it, d, tc});
    }
    try stdout.interface.print("long lived tree of depth {}\t check: {}\n", .{mx, ll});
    try stdout.interface.flush();
}
