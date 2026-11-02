// sha256 — an iterated SHA-256 hash chain: H^N(0), i.e. hash a 32-byte state, feed the
// digest back, N times. Hand-rolled (NO crypto library, so NO SHA-NI hardware path).
// The 32-byte message is 8 big-endian u32 words; output is the 8 digest words.
const std = @import("std");

const K = [64]u32{
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
};

fn rotr(x: u32, n: u5) u32 {
    return (x >> n) | (x << @intCast(32 - @as(u32, n)));
}

// SHA-256 of a 32-byte message given as 8 big-endian u32 words -> 8-word digest.
fn sha256_words(in: [8]u32) [8]u32 {
    const h = [8]u32{ 0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19 };
    var w: [64]u32 = undefined;
    var i: usize = 0;
    while (i < 8) : (i += 1) w[i] = in[i];
    w[8] = 0x80000000;
    i = 9;
    while (i < 15) : (i += 1) w[i] = 0;
    w[15] = 256;
    i = 16;
    while (i < 64) : (i += 1) {
        const s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3);
        const s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16] +% s0 +% w[i - 7] +% s1;
    }
    var a = h[0];
    var b = h[1];
    var c = h[2];
    var d = h[3];
    var e = h[4];
    var f = h[5];
    var g = h[6];
    var hh = h[7];
    i = 0;
    while (i < 64) : (i += 1) {
        const s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
        const ch = (e & f) ^ (~e & g);
        const t1 = hh +% s1 +% ch +% K[i] +% w[i];
        const s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
        const maj = (a & b) ^ (a & c) ^ (b & c);
        const t2 = s0 +% maj;
        hh = g;
        g = f;
        f = e;
        e = d +% t1;
        d = c;
        c = b;
        b = a;
        a = t1 +% t2;
    }
    return [8]u32{
        h[0] +% a, h[1] +% b, h[2] +% c, h[3] +% d,
        h[4] +% e, h[5] +% f, h[6] +% g, h[7] +% hh,
    };
}

pub fn main() !void {
    var arena = std.heap.ArenaAllocator.init(std.heap.smp_allocator);
    defer arena.deinit();
    const alloc = arena.allocator();
    const args = try std.process.argsAlloc(alloc);
    if (args.len < 2) {
        std.debug.print("usage: sha256 <n>\n", .{});
        std.process.exit(1);
    }
    const n = std.fmt.parseInt(i64, args[1], 10) catch 0;
    var state = [8]u32{ 0, 0, 0, 0, 0, 0, 0, 0 };
    var k: i64 = 0;
    while (k < n) : (k += 1) {
        state = sha256_words(state);
    }
    var buf: [256]u8 = undefined;
    var stdout = std.fs.File.stdout().writer(&buf);
    const out = &stdout.interface;
    var i: usize = 0;
    while (i < 8) : (i += 1) {
        try out.print("{}\n", .{state[i]});
    }
    try out.flush();
}
