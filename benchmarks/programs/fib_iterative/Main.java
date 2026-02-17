public class Main {
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        long a = 0, b = 1;
        for (long i = 0; i < n; i++) {
            long temp = b;
            b = (a + b) % 1000000007;
            a = temp;
        }
        System.out.println(a);
    }
}
