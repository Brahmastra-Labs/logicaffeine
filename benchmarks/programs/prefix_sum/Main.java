public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        long seed = 42;
        for (int i = 0; i < n; i++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            arr[i] = ((seed >> 16) & 0x7fff) % 1000;
        }
        for (int i = 1; i < n; i++) arr[i] = (arr[i] + arr[i - 1]) % 1000000007;
        System.out.println(arr[n - 1]);
    }
}
