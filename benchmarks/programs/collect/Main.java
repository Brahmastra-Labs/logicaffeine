// collect — the table main.c hand-rolls: flat (key, value) slots, here interleaved in one long[] (key at 2i, value at
// 2i + 1), a power-of-two capacity at least twice n, Fibonacci hashing (the key times 2^64/phi, its top bits), linear
// probing, key 0 as the empty mark over zeroed memory and the key 0 itself kept beside the table.
public class Main {
    static long[] table;
    static int mask;
    static int shift;
    static boolean hasZero;
    static long zeroValue;

    static int slotOf(long key) {
        return (int) ((key * 0x9E3779B97F4A7C15L) >>> shift);
    }

    static void insert(long key, long value) {
        if (key == 0) { hasZero = true; zeroValue = value; return; }
        int i = slotOf(key);
        while (table[2 * i] != 0 && table[2 * i] != key)
            i = (i + 1) & mask;
        table[2 * i] = key;
        table[2 * i + 1] = value;
    }

    // The slot holding `key`, -1 when it holds none, -2 for the key 0.
    static int find(long key) {
        if (key == 0) return hasZero ? -2 : -1;
        int i = slotOf(key);
        while (table[2 * i] != 0) {
            if (table[2 * i] == key) return i;
            i = (i + 1) & mask;
        }
        return -1;
    }

    static long valueAt(int slot) {
        return slot == -2 ? zeroValue : table[2 * slot + 1];
    }

    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        int bits = 4;
        while ((1L << bits) < 2 * n) bits++;
        int capacity = 1 << bits;
        mask = capacity - 1;
        shift = 64 - bits;
        table = new long[2 * capacity];
        long seed = 42;
        for (long i = 0; i < n; i++) {
            seed = (seed * 1103515245L + 12345) % 2147483648L;
            insert(seed, seed * 2);
        }
        long hit = 42, miss = seed, found = 0, h = 0;
        for (long i = 0; i < n; i++) {
            hit = (hit * 1103515245L + 12345) % 2147483648L;
            int s = find(hit);
            if (s != -1) { found++; h = (h * 31 + valueAt(s)) & 268435455; }
            miss = (miss * 1103515245L + 12345) % 2147483648L;
            s = find(miss);
            if (s != -1) { found++; h = (h * 31 + valueAt(s)) & 268435455; }
        }
        System.out.println(found + " " + h);
    }
}
