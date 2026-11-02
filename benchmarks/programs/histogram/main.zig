const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    var counts = [_]i64{0} ** 1000;
    var seed: i64 = 42;
    var i: i64 = 0;
    while (i < n) : (i += 1) {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        const v: usize = @intCast(@rem((seed >> 16) & 0x7fff, 1000));
        counts[v] += 1;
    }
    var max_freq: i64 = 0;
    var max_idx: i64 = 0;
    for (0..1000) |j| {
        if (counts[j] > max_freq) {
            max_freq = counts[j];
            max_idx = @intCast(j);
        }
    }
    var h: i64 = 0;
    for (counts) |c| {
        h = (h * 31 + c) & 268435455;
    }
    try stdout.interface.print("{} {} {}\n", .{ max_freq, max_idx, h });
    try stdout.interface.flush();
}
