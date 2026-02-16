const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const arg = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, arg, 10);

    var gpa = std.heap.GeneralPurposeAllocator(.{}){};
    const allocator = gpa.allocator();

    var list: std.ArrayList(u8) = .empty;
    defer list.deinit(allocator);

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
    try stdout.interface.print("{}\n", .{spaces});
    try stdout.interface.flush();
}
