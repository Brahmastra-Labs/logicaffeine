const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));

    const allocator = std.heap.page_allocator;
    const text = try allocator.alloc(u8, n);
    defer allocator.free(text);

    var pos: usize = 0;
    while (pos < n) {
        if (pos > 0 and pos % 1000 == 0 and pos + 5 <= n) {
            text[pos] = 'X';
            text[pos + 1] = 'X';
            text[pos + 2] = 'X';
            text[pos + 3] = 'X';
            text[pos + 4] = 'X';
            pos += 5;
        } else {
            text[pos] = @intCast(@as(usize, 'a') + pos % 5);
            pos += 1;
        }
    }
    const needle = "XXXXX";
    var count: i64 = 0;
    var i: usize = 0;
    while (i + needle.len <= n) : (i += 1) {
        if (std.mem.eql(u8, text[i..][0..needle.len], needle)) {
            count += 1;
        }
    }
    try stdout.interface.print("{d}\n", .{count});
    try stdout.interface.flush();
}
