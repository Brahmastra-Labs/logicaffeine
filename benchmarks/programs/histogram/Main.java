public class Main {
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        long[] counts = new long[1000];
        long seed = 42;
        for (long i = 0; i < n; i++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            counts[(int)(((seed >> 16) & 0x7fff) % 1000)]++;
        }
        long maxFreq = 0, maxIdx = 0;
        for (int i = 0; i < 1000; i++) {
            if (counts[i] > maxFreq) { maxFreq = counts[i]; maxIdx = i; }
        }
        long h = 0;
        for (long c : counts) h = (h * 31 + c) & 268435455;
        System.out.println(maxFreq + " " + maxIdx + " " + h);
    }
}
