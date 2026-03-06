public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        long seed = 42;
        for (int i = 0; i < n; i++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            arr[i] = (seed >> 16) % 1000;
        }
        long[] counts = new long[1000];
        for (long v : arr) counts[(int) v]++;
        long[] sorted = new long[n];
        int idx = 0;
        for (int v = 0; v < 1000; v++)
            for (long c = 0; c < counts[v]; c++)
                sorted[idx++] = v;
        long checksum = 0;
        for (long v : sorted) checksum = (checksum + v) % 1000000007;
        System.out.println(sorted[0] + " " + sorted[n - 1] + " " + checksum);
    }
}
