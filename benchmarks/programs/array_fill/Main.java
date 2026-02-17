public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        for (int i = 0; i < n; i++) arr[i] = ((long) i * 7 + 3) % 1000000;
        long sum = 0;
        for (long v : arr) sum = (sum + v) % 1000000007;
        System.out.println(sum);
    }
}
