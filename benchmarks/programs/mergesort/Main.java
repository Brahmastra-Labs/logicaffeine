import java.util.Arrays;

public class Main {
    static void mergeSort(long[] arr) {
        int n = arr.length;
        if (n < 2) return;
        int mid = n / 2;
        long[] left = Arrays.copyOfRange(arr, 0, mid);
        long[] right = Arrays.copyOfRange(arr, mid, n);
        mergeSort(left);
        mergeSort(right);
        int i = 0, j = 0, k = 0;
        while (i < mid && j < n - mid) {
            if (left[i] <= right[j]) arr[k++] = left[i++];
            else arr[k++] = right[j++];
        }
        while (i < mid) arr[k++] = left[i++];
        while (j < n - mid) arr[k++] = right[j++];
    }

    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        long seed = 42;
        for (int i = 0; i < n; i++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            arr[i] = (seed >> 16) & 0x7fff;
        }
        mergeSort(arr);
        long checksum = 0;
        for (long v : arr) checksum = (checksum * 31 + v) & 268435455;
        System.out.println(arr[0] + " " + arr[n - 1] + " " + checksum);
    }
}
