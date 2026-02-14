#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: strings <n>\n"); return 1; }
    int n = atoi(argv[1]);
    size_t cap = 16, len = 0;
    char *buf = malloc(cap);
    if (!buf) return 1;
    char num[32];
    for (int i = 0; i < n; i++) {
        int slen = snprintf(num, sizeof(num), "%d ", i);
        while (len + slen >= cap) {
            cap *= 2;
            buf = realloc(buf, cap);
            if (!buf) return 1;
        }
        memcpy(buf + len, num, slen);
        len += slen;
    }
    int spaces = 0;
    for (size_t i = 0; i < len; i++)
        if (buf[i] == ' ') spaces++;
    printf("%d\n", spaces);
    free(buf);
    return 0;
}
