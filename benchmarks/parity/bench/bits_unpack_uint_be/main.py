import sys
n = int(sys.argv[1]); MOD = 1000000007
b = bytearray([1, 2, 3, 4, 5, 6]); s = 0
for r in range(n):
    b[0] = r % 256
    s = (s + int.from_bytes(bytes(b), 'big')) % MOD
print(s)
