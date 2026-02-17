import java.util.HashSet;
public class Main {
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        long[] arr = new long[(int)n];
        long seed = 42;
        for (int i=0;i<n;i++) { seed=(seed*1103515245+12345)%2147483648L; arr[i]=((seed>>16)&0x7fff)%n; }
        HashSet<Long> seen = new HashSet<>();
        long count = 0;
        for (long x : arr) {
            long c = n - x;
            if (c >= 0 && seen.contains(c)) count++;
            seen.add(x);
        }
        System.out.println(count);
    }
}
