public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] perm1 = new long[n], count = new long[n], perm = new long[n];
        for (int i = 0; i < n; i++) perm1[i] = i;
        long maxFlips = 0, checksum = 0, permCount = 0;
        int r = n;
        outer: while (true) {
            while (r > 1) { count[r-1] = r; r--; }
            System.arraycopy(perm1, 0, perm, 0, n);
            long flips = 0;
            while (perm[0] != 0) {
                int k = (int) perm[0] + 1;
                for (int i = 0; i < k/2; i++) { long t = perm[i]; perm[i] = perm[k-1-i]; perm[k-1-i] = t; }
                flips++;
            }
            if (flips > maxFlips) maxFlips = flips;
            checksum += (permCount % 2 == 0) ? flips : -flips;
            permCount++;
            while (true) {
                if (r == n) break outer;
                long p0 = perm1[0];
                for (int i = 0; i < r; i++) perm1[i] = perm1[i+1];
                perm1[r] = p0;
                if (--count[r] > 0) break;
                r++;
            }
        }
        System.out.println(checksum + "\n" + maxFlips);
    }
}
