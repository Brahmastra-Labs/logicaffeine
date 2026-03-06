public class Main {
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        long count = 0;
        for (long i = 2; i <= n; i++) {
            boolean isPrime = true;
            for (long d = 2; d * d <= i; d++) {
                if (i % d == 0) { isPrime = false; break; }
            }
            if (isPrime) count++;
        }
        System.out.println(count);
    }
}
