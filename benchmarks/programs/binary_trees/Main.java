public class Main {
    static long makeCheck(int d) { return d == 0 ? 1 : 1 + makeCheck(d-1) + makeCheck(d-1); }
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]), mn = 4, mx = Math.max(mn + 2, n);
        System.out.printf("stretch tree of depth %d\t check: %d%n", mx+1, makeCheck(mx+1));
        long ll = makeCheck(mx);
        for (int d = mn; d <= mx; d += 2) {
            int it = 1 << (mx - d + mn); long tc = 0;
            for (int i = 0; i < it; i++) tc += makeCheck(d);
            System.out.printf("%d\t trees of depth %d\t check: %d%n", it, d, tc);
        }
        System.out.printf("long lived tree of depth %d\t check: %d%n", mx, ll);
    }
}
