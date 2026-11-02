#include <cstdio>
#include <cstdlib>
#include <vector>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: bubble_sort <n>\n"); return 1; }
    int n = std::atoi(argv[1]);
    std::vector<long> arr(n);
    unsigned int seed = 42;
    for (int i = 0; i < n; i++) {
        seed = seed * 1103515245 + 12345;
        arr[i] = (seed >> 16) & 0x7fff;
    }
    for (int i = 0; i < n - 1; i++)
        for (int j = 0; j < n - 1 - i; j++)
            if (arr[j] > arr[j + 1])
                std::swap(arr[j], arr[j + 1]);
    long checksum = 0;
    for (int i = 0; i < n; i++) checksum = (checksum * 31 + arr[i]) & 268435455;
    printf("%ld %ld %ld\n", arr[0], arr[n - 1], checksum);
    return 0;
}
