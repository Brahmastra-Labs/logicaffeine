const std = @import("std");
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const alloc = std.heap.smp_allocator;
    var perm1 = try alloc.alloc(i64, n);
    var count_arr = try alloc.alloc(i64, n);
    var perm = try alloc.alloc(i64, n);
    for (0..n) |i| perm1[i] = @intCast(i);
    var maxFlips: i64 = 0;
    var checksum: i64 = 0;
    var permCount: i64 = 0;
    var r: usize = n;
    while (true) {
        while (r > 1) { count_arr[r - 1] = @intCast(r); r -= 1; }
        @memcpy(perm, perm1);
        var flips: i64 = 0;
        while (perm[0] != 0) {
            const k: usize = @intCast(perm[0] + 1);
            for (0..k / 2) |i| {
                const t = perm[i];
                perm[i] = perm[k - 1 - i];
                perm[k - 1 - i] = t;
            }
            flips += 1;
        }
        if (flips > maxFlips) maxFlips = flips;
        if (@rem(permCount, 2) == 0) { checksum += flips; } else { checksum -= flips; }
        permCount += 1;
        while (true) {
            if (r == n) {
                try stdout.interface.print("{}\n{}\n", .{checksum, maxFlips});
                try stdout.interface.flush();
                return;
            }
            const p0 = perm1[0];
            for (0..r) |i| perm1[i] = perm1[i + 1];
            perm1[r] = p0;
            count_arr[r] -= 1;
            if (count_arr[r] > 0) break;
            r += 1;
        }
    }
}
