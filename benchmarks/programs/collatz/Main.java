public class Main {
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        long total = 0;
        for (long i = 1; i <= n; i++) {
            long k = i;
            while (k != 1) {
                if (k % 2 == 0) k /= 2;
                else k = 3 * k + 1;
                total++;
            }
        }
        System.out.println(total);
    }
}
