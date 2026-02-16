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

    var map = std.AutoHashMap(i64, i64).init(allocator);
    defer map.deinit();
    try map.ensureTotalCapacity(@intCast(n));

    var i: i64 = 0;
    while (i < n) : (i += 1) {
        try map.put(i, i * 2);
    }
    var found: i64 = 0;
    i = 0;
    while (i < n) : (i += 1) {
        if (map.get(i)) |v| {
            if (v == i * 2) found += 1;
        }
    }
    try stdout.interface.print("{}\n", .{found});
    try stdout.interface.flush();
}
