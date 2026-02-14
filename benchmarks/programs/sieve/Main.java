public class Main {
    public static void main(String[] args) {
        int limit = Integer.parseInt(args[0]);
        boolean[] sieve = new boolean[limit + 1];
        int count = 0;
        for (int i = 2; i <= limit; i++) {
            if (!sieve[i]) {
                count++;
                for (long j = (long) i * i; j <= limit; j += i)
                    sieve[(int) j] = true;
            }
        }
        System.out.println(count);
    }
}
