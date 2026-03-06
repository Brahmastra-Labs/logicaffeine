#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    char *text = malloc(n + 6);
    long pos = 0;
    while (pos < n) {
        if (pos > 0 && pos % 1000 == 0 && pos + 5 <= n) {
            memcpy(text + pos, "XXXXX", 5);
            pos += 5;
        } else {
            text[pos] = 'a' + (pos % 5);
            pos++;
        }
    }
    text[n] = '\0';
    const char *needle = "XXXXX";
    int needle_len = 5;
    long count = 0;
    for (long i = 0; i <= n - needle_len; i++) {
        int match = 1;
        for (int j = 0; j < needle_len; j++) {
            if (text[i + j] != needle[j]) { match = 0; break; }
        }
        if (match) count++;
    }
    printf("%ld\n", count);
    free(text);
    return 0;
}
