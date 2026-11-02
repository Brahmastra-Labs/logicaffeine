public class Main {
    static long solve(long row, long cols, long diag1, long diag2, long n) {
        if (row == n) return 1;
        long count = 0;
        long available = ((1L << n) - 1) & ~(cols | diag1 | diag2);
        while (available != 0) {
            long bit = available & (-available);
            available ^= bit;
            count += solve(row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1, n);
        }
        return count;
    }
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        System.out.println(solve(0, 0, 0, 0, n));
    }
}
