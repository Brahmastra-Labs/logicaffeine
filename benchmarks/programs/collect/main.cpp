// collect — the table main.c hand-rolls: flat 16-byte (key, value) slots, a power-of-two capacity at least twice n,
// Fibonacci hashing (the key times 2^64/phi, its top bits), linear probing, key 0 as the empty mark over zeroed memory
// and the key 0 itself kept beside the table.
#include <cstdio>
#include <cstdlib>

struct Slot { long key; long value; };

static Slot *table;
static unsigned long mask;
static int shift;
static bool has_zero;
static long zero_value;

static unsigned long slot_of(long key) {
    return ((unsigned long)key * 0x9E3779B97F4A7C15UL) >> shift;
}

static void insert(long key, long value) {
    if (key == 0) { has_zero = true; zero_value = value; return; }
    unsigned long i = slot_of(key);
    while (table[i].key != 0 && table[i].key != key)
        i = (i + 1) & mask;
    table[i].key = key;
    table[i].value = value;
}

static bool find(long key, long &value) {
    if (key == 0) { value = zero_value; return has_zero; }
    unsigned long i = slot_of(key);
    while (table[i].key != 0) {
        if (table[i].key == key) { value = table[i].value; return true; }
        i = (i + 1) & mask;
    }
    return false;
}

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: collect <n>\n"); return 1; }
    long n = std::atol(argv[1]);
    int bits = 4;
    while ((1L << bits) < 2 * n) bits++;
    unsigned long capacity = 1UL << bits;
    mask = capacity - 1;
    shift = 64 - bits;
    table = static_cast<Slot *>(std::calloc(capacity, sizeof(Slot)));
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        insert(seed, seed * 2);
    }
    long hit = 42, miss = seed, found = 0, h = 0, v;
    for (long i = 0; i < n; i++) {
        hit = (hit * 1103515245 + 12345) % 2147483648L;
        if (find(hit, v)) { found++; h = (h * 31 + v) & 268435455; }
        miss = (miss * 1103515245 + 12345) % 2147483648L;
        if (find(miss, v)) { found++; h = (h * 31 + v) & 268435455; }
    }
    printf("%ld %ld\n", found, h);
    std::free(table);
    return 0;
}
