public class Main {
    static long ackermann(long m, long n) {
        if (m == 0) return n + 1;
        if (n == 0) return ackermann(m - 1, 1);
        return ackermann(m - 1, ackermann(m, n - 1));
    }

    public static void main(String[] args) {
        long m = Long.parseLong(args[0]);
        System.out.println(ackermann(3, m));
    }
}
