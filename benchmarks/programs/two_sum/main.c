// two_sum — the set every implementation hand-rolls: flat 8-byte key slots, a power-of-two capacity at least twice n,
// Fibonacci hashing (the key times 2^64/phi, its top bits), linear probing. Key 0 marks an empty slot, so the table
// starts as zeroed memory, and the key 0 itself, which a complement can be, is a flag beside the table.
#include <stdio.h>
#include <stdlib.h>

static long *table;
static unsigned long mask;
static int shift;
static int has_zero;

static unsigned long slot_of(long key) {
    return ((unsigned long)key * 0x9E3779B97F4A7C15UL) >> shift;
}

static int ht_contains(long key) {
    if (key == 0) return has_zero;
    unsigned long h = slot_of(key);
    while (table[h] != 0) {
        if (table[h] == key) return 1;
        h = (h + 1) & mask;
    }
    return 0;
}

static void ht_insert(long key) {
    if (key == 0) { has_zero = 1; return; }
    unsigned long h = slot_of(key);
    while (table[h] != 0) {
        if (table[h] == key) return;
        h = (h + 1) & mask;
    }
    table[h] = key;
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long target = 2147483647L;
    long *arr = malloc(n * sizeof(long));
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        if (i > 0 && (seed >> 28) == 0)
            arr[i] = target - arr[seed % i];
        else
            arr[i] = seed;
    }
    int bits = 4;
    while ((1L << bits) < 2 * n) bits++;
    unsigned long capacity = 1UL << bits;
    mask = capacity - 1;
    shift = 64 - bits;
    table = calloc(capacity, sizeof(long));
    long count = 0;
    for (long i = 0; i < n; i++) {
        long complement = target - arr[i];
        if (ht_contains(complement)) count++;
        ht_insert(arr[i]);
    }
    printf("%ld\n", count);
    free(arr); free(table);
    return 0;
}
