#include <cstdio>
#include <cstdlib>
#include <unordered_map>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: collect <n>\n"); return 1; }
    int n = std::atoi(argv[1]);
    std::unordered_map<int, int> map;
    map.reserve(n);
    for (int i = 0; i < n; i++)
        map[i] = i * 2;
    int found = 0;
    for (int i = 0; i < n; i++)
        if (map[i] == i * 2) found++;
    printf("%d\n", found);
    return 0;
}
