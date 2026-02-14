#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: bubble_sort <n>\n"); return 1; }
    int n = atoi(argv[1]);
    int *arr = malloc(n * sizeof(int));
    if (!arr) return 1;
    unsigned int seed = 42;
    for (int i = 0; i < n; i++) {
        seed = seed * 1103515245 + 12345;
        arr[i] = (seed >> 16) & 0x7fff;
    }
    for (int i = 0; i < n - 1; i++)
        for (int j = 0; j < n - 1 - i; j++)
            if (arr[j] > arr[j + 1]) {
                int tmp = arr[j];
                arr[j] = arr[j + 1];
                arr[j + 1] = tmp;
            }
    printf("%d\n", arr[0]);
    free(arr);
    return 0;
}
