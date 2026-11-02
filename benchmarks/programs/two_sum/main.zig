// two_sum — the set main.c hand-rolls: flat 8-byte key slots, a power-of-two capacity at least twice n, Fibonacci
// hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory (mmap'd
// pages from the page allocator) and the key 0 itself a flag beside the table.
const std = @import("std");

const Set = struct {
    slots: []i64,
    mask: usize,
    shift: u6,
    has_zero: bool = false,

    fn slotOf(self: *const Set, key: i64) usize {
        return @intCast((@as(u64, @bitCast(key)) *% 0x9E3779B97F4A7C15) >> self.shift);
    }

    fn contains(self: *const Set, key: i64) bool {
        if (key == 0) return self.has_zero;
        var h = self.slotOf(key);
        while (self.slots[h] != 0) {
            if (self.slots[h] == key) return true;
            h = (h + 1) & self.mask;
        }
        return false;
    }

    fn insert(self: *Set, key: i64) void {
        if (key == 0) {
            self.has_zero = true;
            return;
        }
        var h = self.slotOf(key);
        while (self.slots[h] != 0) {
            if (self.slots[h] == key) return;
            h = (h + 1) & self.mask;
        }
        self.slots[h] = key;
    }
};

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args_it = std.process.args();
    _ = args_it.skip();
    const n_str = args_it.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    const target: i64 = 2147483647;
    const alloc = std.heap.smp_allocator;
    const nu: usize = @intCast(n);
    var arr = try alloc.alloc(i64, nu);
    var seed: i64 = 42;
    for (0..nu) |i| {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        const ii: i64 = @intCast(i);
        arr[i] = if (ii > 0 and (seed >> 28) == 0) target - arr[@intCast(@rem(seed, ii))] else seed;
    }
    var bits: u6 = 4;
    while ((@as(i64, 1) << bits) < 2 * n) bits += 1;
    const capacity = @as(usize, 1) << bits;
    const slots = try std.heap.page_allocator.alloc(i64, capacity);
    defer std.heap.page_allocator.free(slots);
    var seen = Set{ .slots = slots, .mask = capacity - 1, .shift = @intCast(64 - @as(u7, bits)) };
    var count: i64 = 0;
    for (arr) |x| {
        if (seen.contains(target - x)) count += 1;
        seen.insert(x);
    }
    try stdout.interface.print("{}\n", .{count});
    try stdout.interface.flush();
}
