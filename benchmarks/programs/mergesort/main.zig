const std = @import("std");

fn mergeSort(arr: []i64, tmp: []i64) void {
    if (arr.len < 2) return;
    const mid = arr.len / 2;
    mergeSort(arr[0..mid], tmp[0..mid]);
    mergeSort(arr[mid..], tmp[mid..]);
    var i: usize = 0;
    var j: usize = mid;
    var k: usize = 0;
    while (i < mid and j < arr.len) {
        if (arr[i] <= arr[j]) { tmp[k] = arr[i]; i += 1; } else { tmp[k] = arr[j]; j += 1; }
        k += 1;
    }
    while (i < mid) { tmp[k] = arr[i]; i += 1; k += 1; }
    while (j < arr.len) { tmp[k] = arr[j]; j += 1; k += 1; }
    @memcpy(arr, tmp[0..arr.len]);
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
    const tmp = try allocator.alloc(i64, n);
    defer allocator.free(tmp);
    var seed: i64 = 42;
    for (0..n) |i| {
        seed = @mod(seed *% 1103515245 + 12345, 2147483648);
        arr[i] = (seed >> 16) & 0x7fff;
    }
    mergeSort(arr, tmp);
    var checksum: i64 = 0;
    for (arr) |v| checksum = @mod(checksum + v, 1000000007);
    try stdout.interface.print("{} {} {}\n", .{ arr[0], arr[n - 1], checksum });
    try stdout.interface.flush();
}
