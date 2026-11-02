const std = @import("std");

const MOD: i64 = 1000000007;
const NGROUP: usize = 32768;

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    var seed: i64 = 42;
    var groups = [_]i64{0} ** NGROUP;
    var kept: i64 = 0;
    var r: i64 = 0;
    while (r < n) : (r += 1) {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        const key = (seed >> 16) & 0x7fff;
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        const value = (seed >> 16) & 0x7fff;
        if (@rem(value, 3) == 0) continue;
        const mapped = @rem(value * key + 7, MOD);
        const g: usize = @intCast(@rem(key, @as(i64, NGROUP)));
        groups[g] = @rem(groups[g] + mapped, MOD);
        kept += 1;
    }
    var checksum: i64 = 0;
    var g: usize = 0;
    while (g < NGROUP) : (g += 1) {
        checksum = (checksum * 31 + groups[g]) & 268435455;
    }
    try stdout.interface.print("{}\n{}\n", .{ kept, checksum });
    try stdout.interface.flush();
}
