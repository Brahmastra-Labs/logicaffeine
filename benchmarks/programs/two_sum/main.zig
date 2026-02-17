const std = @import("std");
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args_it = std.process.args();
    _ = args_it.skip();
    const n_str = args_it.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    const alloc = std.heap.page_allocator;
    const nu: usize = @intCast(n);
    var arr = try alloc.alloc(i64, nu);
    var seed: i64 = 42;
    for (0..nu) |i| { seed = @mod(seed *% 1103515245 + 12345, 2147483648); arr[i] = @mod((seed >> 16) & 0x7fff, n); }
    var seen = std.AutoHashMap(i64, void).init(alloc);
    var count: i64 = 0;
    for (arr) |x| {
        const c = n - x;
        if (c >= 0 and seen.contains(c)) count += 1;
        try seen.put(x, {});
    }
    try stdout.interface.print("{}\n", .{count});
    try stdout.interface.flush();
}
