const std = @import("std");
fn A(i: usize, j: usize) f64 { return 1.0 / @as(f64, @floatFromInt((i+j)*(i+j+1)/2+i+1)); }
pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n: usize = @intCast(try std.fmt.parseInt(i64, n_str, 10));
    const alloc = std.heap.smp_allocator;
    var u = try alloc.alloc(f64, n); var v = try alloc.alloc(f64, n); var t = try alloc.alloc(f64, n);
    for (u) |*x| x.* = 1;
    for (0..10) |_| {
        for (0..n) |i| { var s: f64 = 0; for (0..n) |j| s+=A(i,j)*u[j]; t[i]=s; }
        for (0..n) |i| { var s: f64 = 0; for (0..n) |j| s+=A(j,i)*t[j]; v[i]=s; }
        for (0..n) |i| { var s: f64 = 0; for (0..n) |j| s+=A(i,j)*v[j]; t[i]=s; }
        for (0..n) |i| { var s: f64 = 0; for (0..n) |j| s+=A(j,i)*t[j]; u[i]=s; }
    }
    var vBv: f64 = 0; var vv: f64 = 0;
    for (0..n) |i| { vBv += u[i]*v[i]; vv += v[i]*v[i]; }
    try stdout.interface.print("{d:.9}\n", .{@sqrt(vBv/vv)});
    try stdout.interface.flush();
}
