#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CAPACITY 1048576

struct Entry { int key; int value; int occupied; };

static struct Entry table[CAPACITY];

static unsigned int hash(int key) {
    unsigned int k = (unsigned int)key;
    k ^= k >> 16;
    k *= 0x45d9f3b;
    k ^= k >> 16;
    return k & (CAPACITY - 1);
}

static void insert(int key, int value) {
    unsigned int idx = hash(key);
    while (table[idx].occupied && table[idx].key != key)
        idx = (idx + 1) & (CAPACITY - 1);
    table[idx].key = key;
    table[idx].value = value;
    table[idx].occupied = 1;
}

static int lookup(int key) {
    unsigned int idx = hash(key);
    while (table[idx].occupied) {
        if (table[idx].key == key) return table[idx].value;
        idx = (idx + 1) & (CAPACITY - 1);
    }
    return -1;
}

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: collect <n>\n"); return 1; }
    int n = atoi(argv[1]);
    memset(table, 0, sizeof(table));
    for (int i = 0; i < n; i++)
        insert(i, i * 2);
    int found = 0;
    for (int i = 0; i < n; i++)
        if (lookup(i) == i * 2) found++;
    printf("%d\n", found);
    return 0;
}
