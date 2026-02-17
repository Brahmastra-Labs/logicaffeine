public class Main {
    static int solve(int row, int cols, int diag1, int diag2, int n) {
        if (row == n) return 1;
        int count = 0;
        int available = ((1 << n) - 1) & ~(cols | diag1 | diag2);
        while (available != 0) {
            int bit = available & (-available);
            available ^= bit;
            count += solve(row + 1, cols | bit, (diag1 | bit) << 1, (diag2 | bit) >> 1, n);
        }
        return count;
    }
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        System.out.println(solve(0, 0, 0, 0, n));
    }
}
