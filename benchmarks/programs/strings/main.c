// strings — append the decimal text of 0..n-1, each followed by a space, then count the spaces and hash the bytes.
// Every implementation appends an integer's digits without formatting machinery or a per-number allocation: C and Rust
// have no such call in their standard libraries, so both write the digits by hand; the others use theirs (std::to_chars,
// strconv.AppendInt, StringBuilder.append, addInt, bufPrint, String(i)).
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

// Writes v's decimal digits at out, v >= 0, and returns how many.
static size_t put_digits(char *out, long v) {
    char tmp[20];
    size_t k = 0;
    do {
        tmp[k++] = (char)('0' + v % 10);
        v /= 10;
    } while (v);
    for (size_t j = 0; j < k; j++) out[j] = tmp[k - 1 - j];
    return k;
}

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: strings <n>\n"); return 1; }
    long n = atol(argv[1]);
    size_t cap = (size_t)n * 6, len = 0;
    if (cap < 32) cap = 32;
    char *buf = malloc(cap);
    if (!buf) return 1;
    for (long i = 0; i < n; i++) {
        while (len + 21 >= cap) {
            cap *= 2;
            buf = realloc(buf, cap);
            if (!buf) return 1;
        }
        len += put_digits(buf + len, i);
        buf[len++] = ' ';
    }
    long spaces = 0;
    for (size_t i = 0; i < len; i++)
        if (buf[i] == ' ') spaces++;
    long h = 0;
    for (size_t i = 0; i < len; i++)
        h = (h * 31 + (unsigned char)buf[i]) & 268435455;
    printf("%ld %ld\n", spaces, h);
    free(buf);
    return 0;
}
