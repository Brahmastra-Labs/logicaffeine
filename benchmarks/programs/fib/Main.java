public class Main {
    static long fib(long n) {
        if (n < 2) return n;
        return fib(n - 1) + fib(n - 2);
    }

    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        System.out.println(fib(n));
    }
}
