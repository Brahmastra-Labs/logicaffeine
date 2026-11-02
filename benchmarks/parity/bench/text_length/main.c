#include <stdio.h>
#include <stdlib.h>
#include <string.h>
int main(int argc, char** argv){
    long reps = atol(argv[1]);
    long cells = 400;
    long blen = cells * 10;
    char* base = malloc(blen + 200);
    for (long c = 0; c < cells; c++) memcpy(base + c*10, "abcABC012 ", 10);
    base[blen] = 0;
    char* s = malloc(blen + 200);
    long checksum = 0;
    for (long r = 0; r < reps; r++) {
        long pre = r % 100;
        memset(s, 'a', pre);
        memcpy(s + pre, base, blen + 1);
        checksum += (long)strlen(s);
    }
    printf("%ld\n", checksum);
    return 0;
}
