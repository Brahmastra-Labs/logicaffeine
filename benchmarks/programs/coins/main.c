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
    printf("%ld\n", dp[n]);
    free(dp);
    return 0;
}
