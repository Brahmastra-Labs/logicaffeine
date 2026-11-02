// collect — the table main.c hand-rolls: flat 16-byte (key, value) slots, a power-of-two capacity at least twice n,
// Fibonacci hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory
// (mmap'd pages from the page allocator) and the key 0 itself kept beside the table.
const std = @import("std");

const Slot = struct { key: i64, value: i64 };

const Table = struct {
    slots: []Slot,
    mask: usize,
    shift: u6,
    has_zero: bool = false,
    zero_value: i64 = 0,

    fn slotOf(self: *const Table, key: i64) usize {
        return @intCast((@as(u64, @bitCast(key)) *% 0x9E3779B97F4A7C15) >> self.shift);
    }

    fn insert(self: *Table, key: i64, value: i64) void {
        if (key == 0) {
            self.has_zero = true;
            self.zero_value = value;
            return;
        }
        var i = self.slotOf(key);
        while (self.slots[i].key != 0 and self.slots[i].key != key) i = (i + 1) & self.mask;
        self.slots[i] = .{ .key = key, .value = value };
    }

    fn find(self: *const Table, key: i64) ?i64 {
        if (key == 0) return if (self.has_zero) self.zero_value else null;
        var i = self.slotOf(key);
        while (self.slots[i].key != 0) {
            if (self.slots[i].key == key) return self.slots[i].value;
            i = (i + 1) & self.mask;
        }
        return null;
    }
};

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const arg = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, arg, 10);

    var bits: u6 = 4;
    while ((@as(i64, 1) << bits) < 2 * n) bits += 1;
    const capacity = @as(usize, 1) << bits;
    const slots = try std.heap.page_allocator.alloc(Slot, capacity);
    defer std.heap.page_allocator.free(slots);
    var table = Table{ .slots = slots, .mask = capacity - 1, .shift = @intCast(64 - @as(u7, bits)) };

    var seed: i64 = 42;
    var i: i64 = 0;
    while (i < n) : (i += 1) {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        table.insert(seed, seed * 2);
    }
    var hit: i64 = 42;
    var miss: i64 = seed;
    var found: i64 = 0;
    var h: i64 = 0;
    i = 0;
    while (i < n) : (i += 1) {
        hit = @rem(hit *% 1103515245 + 12345, 2147483648);
        if (table.find(hit)) |v| {
            found += 1;
            h = (h * 31 + v) & 268435455;
        }
        miss = @rem(miss *% 1103515245 + 12345, 2147483648);
        if (table.find(miss)) |v| {
            found += 1;
            h = (h * 31 + v) & 268435455;
        }
    }
    try stdout.interface.print("{} {}\n", .{ found, h });
    try stdout.interface.flush();
}
