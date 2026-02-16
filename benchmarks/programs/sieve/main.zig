const std = @import("std");

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const arg = args.next() orelse return;
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
    try stdout.interface.print("{}\n", .{count});
    try stdout.interface.flush();
}
