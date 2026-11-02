# analytics — filter → map → group-by → reduce over N LCG-generated records.
# Reads N from argv[1]; prints two lines (kept count, checksum). Byte-identical
# across every language. Python ints are arbitrary-precision, so the arithmetic
# stays exact with plain %, >>, and *.
import sys

MOD = 1000000007
NGROUP = 32768


def main():
    n = int(sys.argv[1])
    seed = 42
    groups = [0] * NGROUP
    kept = 0
    for _ in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        key = (seed >> 16) & 0x7fff
        seed = (seed * 1103515245 + 12345) % 2147483648
        value = (seed >> 16) & 0x7fff
        if value % 3 == 0:
            continue
        mapped = (value * key + 7) % MOD
        g = key % NGROUP
        groups[g] = (groups[g] + mapped) % MOD
        kept += 1
    checksum = 0
    for g in range(NGROUP):
        checksum = (checksum * 31 + groups[g]) & 268435455
    sys.stdout.write("%d\n%d\n" % (kept, checksum))


if __name__ == "__main__":
    main()
