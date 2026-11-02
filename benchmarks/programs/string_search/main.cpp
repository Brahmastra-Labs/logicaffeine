#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    char *text = new char[n + 6];
    long seed = 42;
    for (long pos = 0; pos < n; pos++) {
        seed = (seed * 1103515245 + 12345) % 2147483648L;
        text[pos] = 'a' + (seed >> 16) % 5;
    }
    text[n] = '\0';
    const char *needle = "cabed";
    int needle_len = 5;
    long count = 0;
    std::vector<long> positions;
    for (long i = 0; i <= n - needle_len; i++) {
        int match = 1;
        for (int j = 0; j < needle_len; j++) {
            if (text[i + j] != needle[j]) { match = 0; break; }
        }
        if (match) { count++; positions.push_back(i); }
    }
    long checksum = 0;
    for (long p : positions) checksum = (checksum * 31 + p) & 268435455;
    printf("%ld %ld\n", count, checksum);
    delete[] text;
    return 0;
}
