const std = @import("std");

const MOD: i64 = 1000000007;
const NREG: usize = 8;

pub fn main() !void {
    var buf: [4096]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    var args = std.process.args();
    _ = args.skip();
    const n_str = args.next() orelse return;
    const n = try std.fmt.parseInt(i64, n_str, 10);
    var seed: i64 = 42;
    var regs: [NREG]i64 = undefined;
    for (0..NREG) |i| regs[i] = @as(i64, @intCast(i)) + 1;
    var k: i64 = 0;
    while (k < n) : (k += 1) {
        seed = @rem(seed *% 1103515245 + 12345, 2147483648);
        const instr = (seed >> 8) & 0x7fffff;
        const op = @rem(instr, 8);
        const dst: usize = @intCast(@rem(@divTrunc(instr, 8), @as(i64, NREG)));
        const a: usize = @intCast(@rem(@divTrunc(instr, 64), @as(i64, NREG)));
        const imm = @rem(@divTrunc(instr, 512), 251);
        switch (op) {
            0 => regs[dst] = imm,
            1 => regs[dst] = @rem(regs[dst] + regs[a], MOD),
            2 => regs[dst] = @rem(regs[dst] + MOD - regs[a], MOD),
            3 => regs[dst] = @rem(regs[dst] * regs[a] + 1, MOD),
            4 => regs[dst] = @rem(regs[dst] + imm, MOD),
            5 => regs[dst] = @rem(regs[dst] * (imm + 1), MOD),
            6 => regs[dst] = regs[a],
            else => regs[dst] = @rem(regs[dst] + regs[a] + imm, MOD),
        }
    }
    var checksum: i64 = 0;
    for (0..NREG) |i| checksum = (checksum * 31 + regs[i]) & 268435455;
    try stdout.interface.print("{}\n", .{checksum});
    try stdout.interface.flush();
}
