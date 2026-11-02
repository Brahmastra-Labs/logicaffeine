# collect — the table main.c hand-rolls: flat (key, value) slots, here one list holding each key beside its value, a
# power-of-two capacity at least twice n, Fibonacci hashing (the key times 2^64/phi, its top 64-bit bits), linear
# probing, key 0 as the empty mark over a zeroed list and the key 0 itself kept beside the table.
import sys

PHI64 = 0x9E3779B97F4A7C15
MASK64 = (1 << 64) - 1


def main():
    n = int(sys.argv[1])
    bits = 4
    while (1 << bits) < 2 * n:
        bits += 1
    mask = (1 << bits) - 1
    shift = 64 - bits
    table = [0] * (2 << bits)
    zero = [False, 0]

    def insert(key, value):
        if key == 0:
            zero[0] = True
            zero[1] = value
            return
        i = ((key * PHI64) & MASK64) >> shift
        while table[2 * i] != 0 and table[2 * i] != key:
            i = (i + 1) & mask
        table[2 * i] = key
        table[2 * i + 1] = value

    def find(key):
        if key == 0:
            return zero[1] if zero[0] else None
        i = ((key * PHI64) & MASK64) >> shift
        while table[2 * i] != 0:
            if table[2 * i] == key:
                return table[2 * i + 1]
            i = (i + 1) & mask
        return None

    seed = 42
    for _ in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        insert(seed, seed * 2)
    hit = 42
    miss = seed
    found = 0
    h = 0
    for _ in range(n):
        hit = (hit * 1103515245 + 12345) % 2147483648
        v = find(hit)
        if v is not None:
            found += 1
            h = (h * 31 + v) & 268435455
        miss = (miss * 1103515245 + 12345) % 2147483648
        v = find(miss)
        if v is not None:
            found += 1
            h = (h * 31 + v) & 268435455
    print(found, h)


if __name__ == "__main__":
    main()
