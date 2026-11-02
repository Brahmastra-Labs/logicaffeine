#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#define MOD 1000000007L

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    int coins[] = {1, 5, 10, 25, 50, 100};
    long *dp = calloc(n + 1, sizeof(long));
    dp[0] = 1;
    for (int c = 0; c < 6; c++) {
        for (long j = coins[c]; j <= n; j++) {
            dp[j] = (dp[j] + dp[j - coins[c]]) % MOD;
        }
    }
    long checksum = 0;
    for (long j = 0; j <= n; j++) checksum = (checksum * 31 + dp[j]) & 268435455;
    printf("%ld %ld\n", dp[n], checksum);
    free(dp);
    return 0;
}
