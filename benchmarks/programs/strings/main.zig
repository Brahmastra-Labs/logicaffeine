const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const arg = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, arg, 10);

    const allocator = std.heap.smp_allocator;

    var list: std.ArrayList(u8) = .empty;
    defer list.deinit(allocator);
    try list.ensureTotalCapacity(allocator, @intCast(n * 6));

    var i: i64 = 0;
    while (i < n) : (i += 1) {
        var intbuf: [20]u8 = undefined;
        const s = try std.fmt.bufPrint(&intbuf, "{}", .{i});
        try list.appendSlice(allocator, s);
        try list.append(allocator, ' ');
    }
    var spaces: i64 = 0;
    for (list.items) |c| {
        if (c == ' ') spaces += 1;
    }
    var h: i64 = 0;
    for (list.items) |c| {
        h = (h * 31 + @as(i64, c)) & 268435455;
    }
    try stdout.interface.print("{} {}\n", .{ spaces, h });
    try stdout.interface.flush();
}
