const std = @import("std");

pub fn main() !void {
    var args = std.process.args();
    _ = args.next();
    const arg = args.next() orelse return error.MissingArgument;
    const limit = try std.fmt.parseInt(usize, arg, 10);

    const allocator = std.heap.page_allocator;
    const sieve = try allocator.alloc(bool, limit + 1);
    defer allocator.free(sieve);
    @memset(sieve, false);

    var count: u64 = 0;
    for (2..limit + 1) |i| {
        if (!sieve[i]) {
            count += 1;
            var j = i * i;
            while (j <= limit) : (j += i) {
                sieve[j] = true;
            }
        }
    }
    var buf: [64]u8 = undefined;
    const s = try std.fmt.bufPrint(&buf, "{}\n", .{count});
    try std.fs.File.stdout().writeAll(s);
}
