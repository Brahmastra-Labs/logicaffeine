const std = @import("std");

fn gcd(a_in: i64, b_in: i64) i64 {
    var a = a_in;
    var b = b_in;
    while (b > 0) {
        const t = b;
        b = @mod(a, b);
        a = t;
    }
    return a;
}

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    var sum: i64 = 0;
    var i: i64 = 1;
    while (i <= n) : (i += 1) {
        var j: i64 = i;
        while (j <= n) : (j += 1) {
            sum += gcd(i, j);
        }
    }
    try stdout.interface.print("{}\n", .{sum});
    try stdout.interface.flush();
}
