const std = @import("std");

fn partition(arr: []i64, lo_in: usize, hi_in: usize) usize {
    const pivot = arr[hi_in];
    var i = lo_in;
    var j = lo_in;
    while (j < hi_in) : (j += 1) {
        if (arr[j] <= pivot) {
            const t = arr[i]; arr[i] = arr[j]; arr[j] = t;
            i += 1;
        }
    }
    const t = arr[i]; arr[i] = arr[hi_in]; arr[hi_in] = t;
    return i;
}

fn qs(arr: []i64, lo: isize, hi: isize) void {
    if (lo < hi) {
        const p: isize = @intCast(partition(arr, @intCast(lo), @intCast(hi)));
        qs(arr, lo, p - 1);
        qs(arr, p + 1, hi);
    }
}

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const allocator = std.heap.page_allocator;
    var arr = try allocator.alloc(i64, n);
    defer allocator.free(arr);
    var seed: i64 = 42;
    for (0..n) |i| {
        seed = @mod(seed *% 1103515245 + 12345, 2147483648);
        arr[i] = (seed >> 16) & 0x7fff;
    }
    qs(arr, 0, @as(isize, @intCast(n)) - 1);
    var checksum: i64 = 0;
    for (arr) |v| checksum = @mod(checksum + v, 1000000007);
    try stdout.interface.print("{} {} {}\n", .{ arr[0], arr[n - 1], checksum });
    try stdout.interface.flush();
}
