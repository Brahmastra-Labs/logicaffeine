const std = @import("std");
const Node = struct { left: ?*Node, right: ?*Node };
fn bottomUpTree(allocator: std.mem.Allocator, d: i32) !*Node {
    var left: ?*Node = null;
    var right: ?*Node = null;
    if (d > 0) {
        left = try bottomUpTree(allocator, d - 1);
        right = try bottomUpTree(allocator, d - 1);
    }
    const node = try allocator.create(Node);
    node.* = .{ .left = left, .right = right };
    return node;
}
fn check(node: *const Node) i64 {
    const l = node.left orelse return 1;
    return 1 + check(l) + check(node.right.?);
}
fn deleteTree(allocator: std.mem.Allocator, node: *Node) void {
    if (node.left) |l| {
        deleteTree(allocator, l);
        deleteTree(allocator, node.right.?);
    }
    allocator.destroy(node);
}
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i32, n_str, 10);
    const allocator = std.heap.smp_allocator;
    const mn: i32 = 4; const mx = if (mn + 2 > n) mn + 2 else n;
    const stretch = try bottomUpTree(allocator, mx+1);
    try stdout.interface.print("stretch tree of depth {}\t check: {}\n", .{mx+1, check(stretch)});
    deleteTree(allocator, stretch);
    const ll = try bottomUpTree(allocator, mx);
    var d = mn;
    while (d <= mx) : (d += 2) {
        const shift: u5 = @intCast(mx - d + mn);
        const it: i32 = @as(i32, 1) << shift;
        var tc: i64 = 0;
        var i: i32 = 0;
        while (i < it) : (i += 1) {
            const tree = try bottomUpTree(allocator, d);
            tc += check(tree);
            deleteTree(allocator, tree);
        }
        try stdout.interface.print("{}\t trees of depth {}\t check: {}\n", .{it, d, tc});
    }
    try stdout.interface.print("long lived tree of depth {}\t check: {}\n", .{mx, check(ll)});
    deleteTree(allocator, ll);
    try stdout.interface.flush();
}
