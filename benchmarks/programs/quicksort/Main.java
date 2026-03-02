public class Main {
    static int partition(long[] arr, int lo, int hi) {
        long pivot = arr[hi]; int i = lo;
        for (int j = lo; j < hi; j++)
            if (arr[j] <= pivot) { long t = arr[i]; arr[i] = arr[j]; arr[j] = t; i++; }
        long t = arr[i]; arr[i] = arr[hi]; arr[hi] = t;
        return i;
    }
    static void qs(long[] arr, int lo, int hi) {
        if (lo < hi) { int p = partition(arr, lo, hi); qs(arr, lo, p-1); qs(arr, p+1, hi); }
    }
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        long seed = 42;
        for (int i = 0; i < n; i++) { seed = (seed*1103515245+12345)%2147483648L; arr[i] = (seed>>16)&0x7fff; }
        qs(arr, 0, n-1);
        long checksum = 0;
        for (long v : arr) checksum = (checksum + v) % 1000000007;
        System.out.println(arr[0] + " " + arr[n-1] + " " + checksum);
    }
}
