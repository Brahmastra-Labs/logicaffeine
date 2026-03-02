#include <stdio.h>
#include <stdlib.h>

typedef struct Entry { long key; int occupied; } Entry;

static unsigned long next_pow2(unsigned long v) {
    v--;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    v |= v >> 32;
    return v + 1;
}

int ht_contains(Entry *table, unsigned long mask, long key) {
    unsigned long h = ((unsigned long)key * 2654435761UL) & mask;
    while (table[h].occupied) {
        if (table[h].key == key) return 1;
        h = (h + 1) & mask;
    }
    return 0;
}

void ht_insert(Entry *table, unsigned long mask, long key) {
    unsigned long h = ((unsigned long)key * 2654435761UL) & mask;
    while (table[h].occupied) {
        if (table[h].key == key) return;
        h = (h + 1) & mask;
    }
    table[h].key = key;
    table[h].occupied = 1;
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    long target = n;
    long *arr = malloc(n * sizeof(long));
    long seed = 42;
    for (long i = 0; i < n; i++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        arr[i] = ((seed >> 16) & 0x7fff) % n;
    }
    unsigned long capacity = next_pow2((unsigned long)(n * 2));
    if (capacity < 16) capacity = 16;
    unsigned long mask = capacity - 1;
    Entry *table = calloc(capacity, sizeof(Entry));
    long count = 0;
    for (long i = 0; i < n; i++) {
        long complement = target - arr[i];
        if (complement >= 0 && ht_contains(table, mask, complement)) count++;
        ht_insert(table, mask, arr[i]);
    }
    printf("%ld\n", count);
    free(arr); free(table);
    return 0;
}
