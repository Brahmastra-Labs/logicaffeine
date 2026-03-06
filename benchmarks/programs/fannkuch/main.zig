const std = @import("std");
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const alloc = std.heap.page_allocator;
    var perm1 = try alloc.alloc(i32, n);
    var count_arr = try alloc.alloc(i32, n);
    var perm = try alloc.alloc(i32, n);
    for (0..n) |i| perm1[i] = @intCast(i);
    var maxFlips: i32 = 0;
    var checksum: i32 = 0;
    var permCount: i32 = 0;
    var r: usize = n;
    while (true) {
        while (r > 1) { count_arr[r - 1] = @intCast(r); r -= 1; }
        @memcpy(perm, perm1);
        var flips: i32 = 0;
        while (perm[0] != 0) {
            const k: usize = @intCast(perm[0] + 1);
            std.mem.reverse(i32, perm[0..k]);
            flips += 1;
        }
        if (flips > maxFlips) maxFlips = flips;
        if (@mod(permCount, 2) == 0) { checksum += flips; } else { checksum -= flips; }
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
