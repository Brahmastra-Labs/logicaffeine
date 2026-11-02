// two_sum — the set main.c hand-rolls: flat 8-byte key slots, a power-of-two capacity at least twice n, Fibonacci
// hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory and the key
// 0 itself a flag beside the table.
public class Main {
    static long[] table;
    static int mask;
    static int shift;
    static boolean hasZero;

    static int slotOf(long key) {
        return (int) ((key * 0x9E3779B97F4A7C15L) >>> shift);
    }

    static boolean contains(long key) {
        if (key == 0) return hasZero;
        int h = slotOf(key);
        while (table[h] != 0) {
            if (table[h] == key) return true;
            h = (h + 1) & mask;
        }
        return false;
    }

    static void insert(long key) {
        if (key == 0) { hasZero = true; return; }
        int h = slotOf(key);
        while (table[h] != 0) {
            if (table[h] == key) return;
            h = (h + 1) & mask;
        }
        table[h] = key;
    }

    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        long target = 2147483647L;
        long[] arr = new long[(int) n];
        long seed = 42;
        for (int i = 0; i < n; i++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            arr[i] = (i > 0 && (seed >> 28) == 0) ? target - arr[(int) (seed % i)] : seed;
        }
        int bits = 4;
        while ((1L << bits) < 2 * n) bits++;
        int capacity = 1 << bits;
        mask = capacity - 1;
        shift = 64 - bits;
        table = new long[capacity];
        long count = 0;
        for (long x : arr) {
            if (contains(target - x)) count++;
            insert(x);
        }
        System.out.println(count);
    }
}
