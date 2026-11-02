#include <cstdio>
#include <cstdlib>
#include <cstring>
int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    long *perm = new long[n], *perm1 = new long[n], *count = new long[n];
    for (int i = 0; i < n; i++) perm1[i] = i;
    long maxFlips = 0, checksum = 0, permCount = 0;
    int r = n;
    while (1) {
        while (r > 1) { count[r-1] = r; r--; }
        memcpy(perm, perm1, n * sizeof(long));
        long flips = 0;
        while (perm[0] != 0) {
            long k = perm[0] + 1;
            for (long i = 0; i < k/2; i++) { long t = perm[i]; perm[i] = perm[k-1-i]; perm[k-1-i] = t; }
            flips++;
        }
        if (flips > maxFlips) maxFlips = flips;
        checksum += (permCount % 2 == 0) ? flips : -flips;
        permCount++;
        while (1) {
            if (r == n) goto done;
            long p0 = perm1[0];
            for (int i = 0; i < r; i++) perm1[i] = perm1[i+1];
            perm1[r] = p0;
            if (--count[r] > 0) break;
            r++;
        }
    }
done:
    printf("%ld\n%ld\n", checksum, maxFlips);
    delete[] perm; delete[] perm1; delete[] count;
    return 0;
}
