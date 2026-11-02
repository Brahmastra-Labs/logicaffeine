# two_sum — the set main.c hand-rolls: flat key slots, a power-of-two capacity at least twice n, Fibonacci hashing (the
# key times 2^64/phi, its top 64-bit bits), linear probing, key 0 as the empty mark over a zeroed list and the key 0
# itself, which a complement can be, a flag beside the table.
import sys

PHI64 = 0x9E3779B97F4A7C15
MASK64 = (1 << 64) - 1


def main():
    n = int(sys.argv[1])
    target = 2147483647
    arr = [0] * n
    seed = 42
    for i in range(n):
        seed = (seed * 1103515245 + 12345) % 2147483648
        arr[i] = target - arr[seed % i] if i > 0 and (seed >> 28) == 0 else seed
    bits = 4
    while (1 << bits) < 2 * n:
        bits += 1
    mask = (1 << bits) - 1
    shift = 64 - bits
    table = [0] * (1 << bits)
    zero = [False]

    def contains(key):
        if key == 0:
            return zero[0]
        h = ((key * PHI64) & MASK64) >> shift
        while table[h] != 0:
            if table[h] == key:
                return True
            h = (h + 1) & mask
        return False

    def insert(key):
        if key == 0:
            zero[0] = True
            return
        h = ((key * PHI64) & MASK64) >> shift
        while table[h] != 0:
            if table[h] == key:
                return
            h = (h + 1) & mask
        table[h] = key

    count = 0
    for x in arr:
        if contains(target - x):
            count += 1
        insert(x)
    print(count)


if __name__ == "__main__":
    main()
