public class Main {
    static void merge(long[] arr, long[] tmp, int lo, int mid, int hi) {
        int i = lo, j = mid, k = lo;
        while (i < mid && j < hi) {
            if (arr[i] <= arr[j]) tmp[k++] = arr[i++];
            else tmp[k++] = arr[j++];
        }
        while (i < mid) tmp[k++] = arr[i++];
        while (j < hi) tmp[k++] = arr[j++];
        System.arraycopy(tmp, lo, arr, lo, hi - lo);
    }

    static void mergeSort(long[] arr, long[] tmp, int lo, int hi) {
        if (hi - lo < 2) return;
        int mid = lo + (hi - lo) / 2;
        mergeSort(arr, tmp, lo, mid);
        mergeSort(arr, tmp, mid, hi);
        merge(arr, tmp, lo, mid, hi);
    }

    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        long seed = 42;
        for (int i = 0; i < n; i++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            arr[i] = (seed >> 16) & 0x7fff;
        }
        mergeSort(arr, new long[n], 0, n);
        long checksum = 0;
        for (long v : arr) checksum = (checksum + v) % 1000000007;
        System.out.println(arr[0] + " " + arr[n - 1] + " " + checksum);
    }
}
