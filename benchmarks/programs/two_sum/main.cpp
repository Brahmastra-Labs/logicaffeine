// two_sum — the set main.c hand-rolls: flat 8-byte key slots, a power-of-two capacity at least twice n, Fibonacci
// hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory and the key
// 0 itself a flag beside the table.
#include <cstdio>
#include <cstdlib>

static long *table;
static unsigned long mask;
static int shift;
static bool has_zero;

static unsigned long slot_of(long key) {
    return ((unsigned long)key * 0x9E3779B97F4A7C15UL) >> shift;
}

static bool ht_contains(long key) {
    if (key == 0) return has_zero;
    unsigned long h = slot_of(key);
    while (table[h] != 0) {
        if (table[h] == key) return true;
        h = (h + 1) & mask;
    }
    return false;
}

static void ht_insert(long key) {
    if (key == 0) { has_zero = true; return; }
    unsigned long h = slot_of(key);
    while (table[h] != 0) {
        if (table[h] == key) return;
        h = (h + 1) & mask;
    }
    table[h] = key;
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = std::atol(argv[1]);
    long target = 2147483647L;
    long *arr = new long[n];
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        arr[i] = (i > 0 && (seed >> 28) == 0) ? target - arr[seed % i] : seed;
    }
    int bits = 4;
    while ((1L << bits) < 2 * n) bits++;
    unsigned long capacity = 1UL << bits;
    mask = capacity - 1;
    shift = 64 - bits;
    table = static_cast<long *>(std::calloc(capacity, sizeof(long)));
    long count = 0;
    for (long i = 0; i < n; i++) {
        if (ht_contains(target - arr[i])) count++;
        ht_insert(arr[i]);
    }
    printf("%ld\n", count);
    delete[] arr;
    std::free(table);
    return 0;
}
