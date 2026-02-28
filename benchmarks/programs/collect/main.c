#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct Entry { int key; int value; int occupied; };

static unsigned int next_pow2(unsigned int v) {
    v--;
    v |= v >> 1;
    v |= v >> 2;
    v |= v >> 4;
    v |= v >> 8;
    v |= v >> 16;
    return v + 1;
}

static struct Entry *table;
static unsigned int capacity;
static unsigned int mask;

static unsigned int hash(int key) {
    unsigned int k = (unsigned int)key;
    k ^= k >> 16;
    k *= 0x45d9f3b;
    k ^= k >> 16;
    return k & mask;
}

static void insert(int key, int value) {
    unsigned int idx = hash(key);
    while (table[idx].occupied && table[idx].key != key)
        idx = (idx + 1) & mask;
    table[idx].key = key;
    table[idx].value = value;
    table[idx].occupied = 1;
}

static int lookup(int key) {
    unsigned int idx = hash(key);
    while (table[idx].occupied) {
        if (table[idx].key == key) return table[idx].value;
        idx = (idx + 1) & mask;
    }
    return -1;
}

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: collect <n>\n"); return 1; }
    int n = atoi(argv[1]);
    capacity = next_pow2((unsigned int)(n * 2));
    if (capacity < 16) capacity = 16;
    mask = capacity - 1;
    table = (struct Entry *)calloc(capacity, sizeof(struct Entry));
    for (int i = 0; i < n; i++)
        insert(i, i * 2);
    int found = 0;
    for (int i = 0; i < n; i++)
        if (lookup(i) == i * 2) found++;
    printf("%d\n", found);
    free(table);
    return 0;
}
