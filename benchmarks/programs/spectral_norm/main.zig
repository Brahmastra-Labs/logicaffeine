const std = @import("std");
fn A(i: usize, j: usize) f64 { return 1.0 / @as(f64, @floatFromInt((i+j)*(i+j+1)/2+i+1)); }
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const alloc = std.heap.page_allocator;
    var u = try alloc.alloc(f64, n); var v = try alloc.alloc(f64, n); var t = try alloc.alloc(f64, n);
    for (u) |*x| x.* = 1;
    for (0..10) |_| {
        for (0..n) |i| { t[i]=0; for (0..n) |j| t[i]+=A(i,j)*u[j]; }
        for (0..n) |i| { v[i]=0; for (0..n) |j| v[i]+=A(j,i)*t[j]; }
        for (0..n) |i| { t[i]=0; for (0..n) |j| t[i]+=A(i,j)*v[j]; }
        for (0..n) |i| { u[i]=0; for (0..n) |j| u[i]+=A(j,i)*t[j]; }
    }
    var vBv: f64 = 0; var vv: f64 = 0;
    for (0..n) |i| { vBv += u[i]*v[i]; vv += v[i]*v[i]; }
    try stdout.interface.print("{d:.9}\n", .{@sqrt(vBv/vv)});
    try stdout.interface.flush();
}
