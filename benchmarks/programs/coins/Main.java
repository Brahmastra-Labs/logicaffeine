public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        int[] coins = {1, 5, 10, 25, 50, 100};
        long[] dp = new long[n + 1];
        dp[0] = 1;
        for (int c : coins) for (int j = c; j <= n; j++) dp[j] = (dp[j] + dp[j - c]) % 1000000007;
        long checksum = 0;
        for (long v : dp) checksum = (checksum * 31 + v) & 268435455;
        System.out.println(dp[n] + " " + checksum);
    }
}
