const std = @import("std");

pub fn main() !void {
    const stdout = std.io.getStdOut().writer();
    var args = std.process.args();
    _ = args.skip();
    const arg = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, arg, 10);

    var gpa: std.heap.GeneralPurposeAllocator(.{}) = .init;
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
    try stdout.print("{}\n", .{spaces});
}
