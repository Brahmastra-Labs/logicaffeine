const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));

    const allocator = std.heap.smp_allocator;
    const text = try allocator.alloc(u8, n);
    defer allocator.free(text);

    var seed: i64 = 42;
    for (0..n) |pos| {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        text[pos] = @intCast('a' + @rem(seed >> 16, 5));
    }
    const needle = "cabed";
    var count: i64 = 0;
    var positions: std.ArrayList(i64) = .empty;
    defer positions.deinit(allocator);
    var i: usize = 0;
    while (i + needle.len <= n) : (i += 1) {
        var matched = true;
        for (0..needle.len) |j| {
            if (text[i + j] != needle[j]) {
                matched = false;
                break;
            }
        }
        if (matched) {
            count += 1;
            try positions.append(allocator, @intCast(i));
        }
    }
    var checksum: i64 = 0;
    for (positions.items) |p| checksum = (checksum * 31 + p) & 268435455;
    try stdout.interface.print("{d} {d}\n", .{ count, checksum });
    try stdout.interface.flush();
}
