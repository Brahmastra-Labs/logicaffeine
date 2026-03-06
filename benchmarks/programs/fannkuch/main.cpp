#include <cstdio>
#include <cstdlib>
#include <cstring>
int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    int *perm = new int[n], *perm1 = new int[n], *count = new int[n];
    for (int i = 0; i < n; i++) perm1[i] = i;
    int maxFlips = 0, checksum = 0, permCount = 0, r = n;
    while (1) {
        while (r > 1) { count[r-1] = r; r--; }
        memcpy(perm, perm1, n * sizeof(int));
        int flips = 0;
        while (perm[0] != 0) {
            int k = perm[0] + 1;
            for (int i = 0; i < k/2; i++) { int t = perm[i]; perm[i] = perm[k-1-i]; perm[k-1-i] = t; }
            flips++;
        }
        if (flips > maxFlips) maxFlips = flips;
        checksum += (permCount % 2 == 0) ? flips : -flips;
        permCount++;
        while (1) {
            if (r == n) goto done;
            int p0 = perm1[0];
            for (int i = 0; i < r; i++) perm1[i] = perm1[i+1];
            perm1[r] = p0;
            if (--count[r] > 0) break;
            r++;
        }
    }
done:
    printf("%d\n%d\n", checksum, maxFlips);
    delete[] perm; delete[] perm1; delete[] count;
    return 0;
}
