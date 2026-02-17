public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        int capacity = n * 5;
        long[] prev = new long[capacity + 1];
        long[] curr = new long[capacity + 1];
        for (int i = 0; i < n; i++) {
            int w = (i * 17 + 3) % 50 + 1;
            long v = (i * 31 + 7) % 100 + 1;
            for (int j = 0; j <= capacity; j++) {
                curr[j] = prev[j];
                if (j >= w && prev[j - w] + v > curr[j]) curr[j] = prev[j - w] + v;
            }
            long[] t = prev; prev = curr; curr = t;
        }
        System.out.println(prev[capacity]);
    }
}
