// sha256 — an iterated SHA-256 hash chain: H^N(0), hand-rolled (NO MessageDigest,
// NO crypto library, so NO SHA-NI hardware path). 32-byte state = 8 big-endian u32
// words; output is the 8 digest words as unsigned 32-bit decimals.
public class Main {
    static final int[] K = {
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
    };

    static final int[] H = {0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19};
    static final int[] W = new int[64];

    static int rotr(int x, int n) {
        return (x >>> n) | (x << (32 - n));
    }

    // SHA-256 of a 32-byte message given as 8 big-endian u32 words -> 8-word digest.
    static void sha256Words(int[] in, int[] out) {
        int[] h = H;
        int[] w = W;
        for (int i = 0; i < 8; i++) w[i] = in[i];
        w[8] = 0x80000000;                        // the 0x80 pad byte after 32 bytes
        for (int i = 9; i < 15; i++) w[i] = 0;
        w[15] = 256;                              // message length in bits
        for (int i = 16; i < 64; i++) {
            int s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >>> 3);
            int s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >>> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }
        int a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
        for (int i = 0; i < 64; i++) {
            int S1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25);
            int ch = (e & f) ^ (~e & g);
            int t1 = hh + S1 + ch + K[i] + w[i];
            int S0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22);
            int maj = (a & b) ^ (a & c) ^ (b & c);
            int t2 = S0 + maj;
            hh = g; g = f; f = e; e = d + t1; d = c; c = b; b = a; a = t1 + t2;
        }
        out[0] = h[0] + a; out[1] = h[1] + b; out[2] = h[2] + c; out[3] = h[3] + d;
        out[4] = h[4] + e; out[5] = h[5] + f; out[6] = h[6] + g; out[7] = h[7] + hh;
    }

    public static void main(String[] args) {
        if (args.length < 1) {
            System.err.println("usage: sha256 <n>");
            System.exit(1);
        }
        long n = Long.parseLong(args[0]);
        int[] state = {0, 0, 0, 0, 0, 0, 0, 0};
        int[] next = new int[8];
        for (long k = 0; k < n; k++) {
            sha256Words(state, next);
            for (int i = 0; i < 8; i++) state[i] = next[i];
        }
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < 8; i++) {
            sb.append(Integer.toUnsignedString(state[i])).append('\n');
        }
        System.out.print(sb);
    }
}
