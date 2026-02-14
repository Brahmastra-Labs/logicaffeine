#include <cstdio>
#include <cstdlib>
#include <string>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: strings <n>\n"); return 1; }
    int n = std::atoi(argv[1]);
    std::string result;
    result.reserve(n * 6);
    for (int i = 0; i < n; i++) {
        result += std::to_string(i);
        result += ' ';
    }
    int spaces = 0;
    for (char c : result)
        if (c == ' ') spaces++;
    printf("%d\n", spaces);
    return 0;
}
