public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        long[] arr = new long[n];
        int seed = 42;
        for (int i = 0; i < n; i++) {
            seed = seed * 1103515245 + 12345;
            arr[i] = (seed >>> 16) & 0x7fff;
        }
        for (int i = 0; i < n - 1; i++)
            for (int j = 0; j < n - 1 - i; j++)
                if (arr[j] > arr[j + 1]) {
                    long tmp = arr[j];
                    arr[j] = arr[j + 1];
                    arr[j + 1] = tmp;
                }
        long checksum = 0;
        for (long v : arr) checksum = (checksum * 31 + v) & 268435455;
        System.out.println(arr[0] + " " + arr[n - 1] + " " + checksum);
    }
}
