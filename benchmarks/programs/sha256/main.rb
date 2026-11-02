# sha256 — an iterated SHA-256 hash chain: H^N(0), hand-rolled (NO digest/openssl, so
# NO SHA-NI hardware path). 32-byte state = 8 big-endian u32 words; output is the 8
# digest words as unsigned 32-bit decimals. Ruby ints are unbounded, so mask with
# & 0xFFFFFFFF after each op to stay in u32.

MASK = 0xFFFFFFFF

K = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2
].freeze

H = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19].freeze
W = Array.new(64, 0)

def rotr(x, n)
  ((x >> n) | (x << (32 - n))) & MASK
end

# SHA-256 of a 32-byte message given as 8 big-endian u32 words -> 8-word digest.
def sha256_words(inw)
  h = H
  w = W
  (0...8).each { |i| w[i] = inw[i] }
  w[8] = 0x80000000                               # the 0x80 pad byte after 32 bytes
  (9...15).each { |i| w[i] = 0 }
  w[15] = 256                                     # message length in bits
  (16...64).each do |i|
    s0 = rotr(w[i - 15], 7) ^ rotr(w[i - 15], 18) ^ (w[i - 15] >> 3)
    s1 = rotr(w[i - 2], 17) ^ rotr(w[i - 2], 19) ^ (w[i - 2] >> 10)
    w[i] = (w[i - 16] + s0 + w[i - 7] + s1) & MASK
  end
  a, b, c, d, e, f, g, hh = h[0], h[1], h[2], h[3], h[4], h[5], h[6], h[7]
  (0...64).each do |i|
    s1 = rotr(e, 6) ^ rotr(e, 11) ^ rotr(e, 25)
    ch = (e & f) ^ ((~e & MASK) & g)
    t1 = (hh + s1 + ch + K[i] + w[i]) & MASK
    s0 = rotr(a, 2) ^ rotr(a, 13) ^ rotr(a, 22)
    maj = (a & b) ^ (a & c) ^ (b & c)
    t2 = (s0 + maj) & MASK
    hh = g
    g = f
    f = e
    e = (d + t1) & MASK
    d = c
    c = b
    b = a
    a = (t1 + t2) & MASK
  end
  [
    (h[0] + a) & MASK, (h[1] + b) & MASK, (h[2] + c) & MASK, (h[3] + d) & MASK,
    (h[4] + e) & MASK, (h[5] + f) & MASK, (h[6] + g) & MASK, (h[7] + hh) & MASK
  ]
end

if ARGV.length < 1
  warn 'usage: sha256 <n>'
  exit 1
end

n = Integer(ARGV[0])
state = [0, 0, 0, 0, 0, 0, 0, 0]
n.times do
  state = sha256_words(state)
end
out = String.new
(0...8).each { |i| out << state[i].to_s << "\n" }
$stdout.write(out)
