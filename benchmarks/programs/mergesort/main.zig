const std = @import("std");

fn mergeSort(arr: []i64, allocator: std.mem.Allocator) void {
    if (arr.len < 2) return;
    const mid = arr.len / 2;
    const left = allocator.alloc(i64, mid) catch return;
    defer allocator.free(left);
    const right = allocator.alloc(i64, arr.len - mid) catch return;
    defer allocator.free(right);
    @memcpy(left, arr[0..mid]);
    @memcpy(right, arr[mid..]);
    mergeSort(left, allocator);
    mergeSort(right, allocator);
    var i: usize = 0;
    var j: usize = 0;
    var k: usize = 0;
    while (i < mid and j < arr.len - mid) {
        if (left[i] <= right[j]) { arr[k] = left[i]; i += 1; } else { arr[k] = right[j]; j += 1; }
        k += 1;
    }
    while (i < mid) { arr[k] = left[i]; i += 1; k += 1; }
    while (j < arr.len - mid) { arr[k] = right[j]; j += 1; k += 1; }
}

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const allocator = std.heap.page_allocator;
    var arr = try allocator.alloc(i64, n);
    defer allocator.free(arr);
    var seed: i64 = 42;
    for (0..n) |i| {
        seed = @mod(seed *% 1103515245 + 12345, 2147483648);
        arr[i] = (seed >> 16) & 0x7fff;
    }
    mergeSort(arr, allocator);
    var checksum: i64 = 0;
    for (arr) |v| checksum = @mod(checksum + v, 1000000007);
    try stdout.interface.print("{} {} {}\n", .{ arr[0], arr[n - 1], checksum });
    try stdout.interface.flush();
}
