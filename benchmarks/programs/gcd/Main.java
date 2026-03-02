public class Main {
    static long gcd(long a, long b) {
        while (b > 0) { long t = b; b = a % b; a = t; }
        return a;
    }
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        long sum = 0;
        for (long i = 1; i <= n; i++)
            for (long j = i; j <= n; j++)
                sum += gcd(i, j);
        System.out.println(sum);
    }
}
