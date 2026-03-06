public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        long seed = 42;
        for (int i = 0; i < n; i++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            arr[i] = (seed >> 16) & 0x7fff;
        }
        int lo = 0, hi = n - 1;
        while (lo < hi) {
            long tmp = arr[lo]; arr[lo] = arr[hi]; arr[hi] = tmp;
            lo++; hi--;
        }
        System.out.println(arr[0] + " " + arr[n - 1] + " " + arr[n / 2]);
    }
}
