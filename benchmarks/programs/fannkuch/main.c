#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    int *perm = malloc(n * sizeof(int));
    int *perm1 = malloc(n * sizeof(int));
    int *count = malloc(n * sizeof(int));
    int *tmp = malloc(n * sizeof(int));
    for (int i = 0; i < n; i++) perm1[i] = i;
    int max_flips = 0, checksum = 0, perm_count = 0;
    int r = n;
    while (1) {
        while (r > 1) { count[r - 1] = r; r--; }
        memcpy(perm, perm1, n * sizeof(int));
        int flips = 0;
        while (perm[0] != 0) {
            int k = perm[0] + 1;
            for (int i = 0; i < k / 2; i++) {
                int t = perm[i]; perm[i] = perm[k - 1 - i]; perm[k - 1 - i] = t;
            }
            flips++;
        }
        if (flips > max_flips) max_flips = flips;
        checksum += (perm_count % 2 == 0) ? flips : -flips;
        perm_count++;
        while (1) {
            if (r == n) goto done;
            int perm0 = perm1[0];
            for (int i = 0; i < r; i++) perm1[i] = perm1[i + 1];
            perm1[r] = perm0;
            count[r]--;
            if (count[r] > 0) break;
            r++;
        }
    }
done:
    printf("%d\n%d\n", checksum, max_flips);
    free(perm); free(perm1); free(count); free(tmp);
    return 0;
}
