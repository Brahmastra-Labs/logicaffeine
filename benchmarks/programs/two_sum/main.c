#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define TABLE_SIZE 262144
#define MASK (TABLE_SIZE - 1)

typedef struct Entry { long key; int occupied; } Entry;

int ht_contains(Entry *table, long key) {
    long h = (key * 2654435761L) & MASK;
    while (table[h].occupied) {
        if (table[h].key == key) return 1;
        h = (h + 1) & MASK;
    }
    return 0;
}

void ht_insert(Entry *table, long key) {
    long h = (key * 2654435761L) & MASK;
    while (table[h].occupied) {
        if (table[h].key == key) return;
        h = (h + 1) & MASK;
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
    Entry *table = calloc(TABLE_SIZE, sizeof(Entry));
    long count = 0;
    for (long i = 0; i < n; i++) {
        long complement = target - arr[i];
        if (complement >= 0 && ht_contains(table, complement)) count++;
        ht_insert(table, arr[i]);
    }
    printf("%ld\n", count);
    free(arr); free(table);
    return 0;
}
