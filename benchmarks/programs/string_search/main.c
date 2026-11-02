#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    char *text = malloc(n + 6);
    long seed = 42;
    for (long pos = 0; pos < n; pos++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        text[pos] = 'a' + (seed >> 16) % 5;
    }
    text[n] = '\0';
    const char *needle = "cabed";
    int needle_len = 5;
    long count = 0;
    long cap = 1024;
    long *positions = malloc(cap * sizeof(long));
    for (long i = 0; i <= n - needle_len; i++) {
        int match = 1;
        for (int j = 0; j < needle_len; j++) {
            if (text[i + j] != needle[j]) { match = 0; break; }
        }
        if (match) {
            if (count == cap) { cap *= 2; positions = realloc(positions, cap * sizeof(long)); }
            positions[count++] = i;
        }
    }
    long checksum = 0;
    for (long k = 0; k < count; k++) checksum = (checksum * 31 + positions[k]) & 268435455;
    printf("%ld %ld\n", count, checksum);
    free(positions);
    free(text);
    return 0;
}
