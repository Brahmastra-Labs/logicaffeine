const std = @import("std");

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
    var lo: usize = 0;
    var hi: usize = n - 1;
    while (lo < hi) {
        const tmp = arr[lo];
        arr[lo] = arr[hi];
        arr[hi] = tmp;
        lo += 1;
        hi -= 1;
    }
    try stdout.interface.print("{} {} {}\n", .{ arr[0], arr[n - 1], arr[n / 2] });
    try stdout.interface.flush();
}
