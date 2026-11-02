#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char** argv) {
    long reps = atol(argv[1]);
    long blen = 400 * 10;
    char* base = malloc(blen + 1);
    for (long c = 0; c < 400; c++) memcpy(base + c*10, "abcABC012 ", 10);
    base[blen] = 0;
    char* s = malloc(blen + 200);
    long checksum = 0;
    for (long r = 0; r < reps; r++) {
        long pre = r % 100;
        memset(s, 'a', pre);
        memcpy(s + pre, base, blen);
        long len = pre + blen;
        long idx = -1;
        for (long i = 0; i < len; i++) if ((unsigned char)s[i] == 48) { idx = i; break; }
        if (idx >= 0) checksum += idx + 1;
    }
    printf("%ld\n", checksum);
    return 0;
}
