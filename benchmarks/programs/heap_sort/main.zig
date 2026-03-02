const std = @import("std");

fn siftDown(arr: []i64, start_in: usize, end_in: usize) void {
    var root = start_in;
    while (2 * root + 1 <= end_in) {
        const child = 2 * root + 1;
        var sw = root;
        if (arr[sw] < arr[child]) sw = child;
        if (child + 1 <= end_in and arr[sw] < arr[child + 1]) sw = child + 1;
        if (sw == root) return;
        const t = arr[root]; arr[root] = arr[sw]; arr[sw] = t;
        root = sw;
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
    var s: isize = @divTrunc(@as(isize, @intCast(n)) - 2, 2);
    while (s >= 0) : (s -= 1) siftDown(arr, @intCast(s), n - 1);
    var e: usize = n - 1;
    while (e > 0) : (e -= 1) {
        const t = arr[0]; arr[0] = arr[e]; arr[e] = t;
        siftDown(arr, 0, e - 1);
    }
    var checksum: i64 = 0;
    for (arr) |v| checksum = @mod(checksum + v, 1000000007);
    try stdout.interface.print("{} {} {}\n", .{ arr[0], arr[n - 1], checksum });
    try stdout.interface.flush();
}
