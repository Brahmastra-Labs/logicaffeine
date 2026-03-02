public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long MOD = 1000000007L;
        long[] a = new long[n * n];
        long[] b = new long[n * n];
        long[] c = new long[n * n];
        for (int i = 0; i < n; i++)
            for (int j = 0; j < n; j++) {
                a[i * n + j] = (i * n + j) % 100;
                b[i * n + j] = (j * n + i) % 100;
            }
        for (int i = 0; i < n; i++)
            for (int k = 0; k < n; k++)
                for (int j = 0; j < n; j++)
                    c[i * n + j] = (c[i * n + j] + a[i * n + k] * b[k * n + j]) % MOD;
        long checksum = 0;
        for (long v : c) checksum = (checksum + v) % MOD;
        System.out.println(checksum);
    }
}
