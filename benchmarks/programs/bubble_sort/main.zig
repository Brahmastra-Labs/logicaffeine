const std = @import("std");

pub fn main() !void {
    var args = std.process.args();
    _ = args.next();
    const arg = args.next() orelse return error.MissingArgument;
    const n = try std.fmt.parseInt(usize, arg, 10);

    const allocator = std.heap.page_allocator;
    const arr = try allocator.alloc(i32, n);
    defer allocator.free(arr);

    var seed: u32 = 42;
    for (0..n) |i| {
        seed = seed *% 1103515245 +% 12345;
        arr[i] = @intCast((seed >> 16) & 0x7fff);
    }
    for (0..n -| 1) |i| {
        for (0..n - 1 - i) |j| {
            if (arr[j] > arr[j + 1]) {
                const tmp = arr[j];
                arr[j] = arr[j + 1];
                arr[j + 1] = tmp;
            }
        }
    }
    var buf: [64]u8 = undefined;
    const s = try std.fmt.bufPrint(&buf, "{}\n", .{arr[0]});
    try std.fs.File.stdout().writeAll(s);
}
