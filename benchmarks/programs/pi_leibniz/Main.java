public class Main {
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        double sum = 0, sign = 1;
        for (long k = 0; k < n; k++) { sum += sign / (2.0 * k + 1.0); sign = -sign; }
        System.out.printf("%.15f%n", sum * 4.0);
    }
}
