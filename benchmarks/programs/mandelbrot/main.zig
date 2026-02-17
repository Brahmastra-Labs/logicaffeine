const std = @import("std");
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i32, n_str, 10);
    const nf: f64 = @floatFromInt(n);
    var count: i32 = 0;
    var y: i32 = 0;
    while (y < n) : (y += 1) {
        var x: i32 = 0;
        while (x < n) : (x += 1) {
            const cr = 2.0 * @as(f64, @floatFromInt(x)) / nf - 1.5;
            const ci = 2.0 * @as(f64, @floatFromInt(y)) / nf - 1.0;
            var zr: f64 = 0; var zi: f64 = 0; var inside = true;
            for (0..50) |_| {
                const t = zr*zr - zi*zi + cr; zi = 2*zr*zi + ci; zr = t;
                if (zr*zr + zi*zi > 4) { inside = false; break; }
            }
            if (inside) count += 1;
        }
    }
    try stdout.interface.print("{}\n", .{count});
    try stdout.interface.flush();
}
