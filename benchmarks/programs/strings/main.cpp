// strings — append the decimal text of 0..n-1, each followed by a space, then count the spaces and hash the bytes. The
// digits go straight into the string through std::to_chars: no formatting machinery, no per-number allocation.
#include <charconv>
#include <cstdio>
#include <cstdlib>
#include <string>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: strings <n>\n"); return 1; }
    long n = std::atol(argv[1]);
    std::string result;
    result.reserve(n * 6);
    char num[24];
    for (long i = 0; i < n; i++) {
        char *end = std::to_chars(num, num + sizeof(num), i).ptr;
        result.append(num, end);
        result += ' ';
    }
    long spaces = 0;
    for (char c : result)
        if (c == ' ') spaces++;
    long h = 0;
    for (unsigned char c : result)
        h = (h * 31 + c) & 268435455;
    printf("%ld %ld\n", spaces, h);
    return 0;
}
