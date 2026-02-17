public class Main {
    static void siftDown(long[] arr, int start, int end) {
        int root = start;
        while (2 * root + 1 <= end) {
            int child = 2 * root + 1;
            int swap = root;
            if (arr[swap] < arr[child]) swap = child;
            if (child + 1 <= end && arr[swap] < arr[child + 1]) swap = child + 1;
            if (swap == root) return;
            long t = arr[root]; arr[root] = arr[swap]; arr[swap] = t;
            root = swap;
        }
    }
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        long seed = 42;
        for (int i = 0; i < n; i++) { seed = (seed*1103515245+12345)%2147483648L; arr[i] = (seed>>16)&0x7fff; }
        for (int s = (n-2)/2; s >= 0; s--) siftDown(arr, s, n-1);
        for (int end = n-1; end > 0; end--) {
            long t = arr[0]; arr[0] = arr[end]; arr[end] = t;
            siftDown(arr, 0, end-1);
        }
        long checksum = 0;
        for (long v : arr) checksum = (checksum + v) % 1000000007;
        System.out.println(arr[0] + " " + arr[n-1] + " " + checksum);
    }
}
