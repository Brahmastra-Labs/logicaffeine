public class Main {
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        long[] counts = new long[1000];
        long seed = 42;
        for (long i = 0; i < n; i++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            counts[(int)(((seed >> 16) & 0x7fff) % 1000)]++;
        }
        long maxFreq = 0, maxIdx = 0, distinct = 0;
        for (int i = 0; i < 1000; i++) {
            if (counts[i] > 0) distinct++;
            if (counts[i] > maxFreq) { maxFreq = counts[i]; maxIdx = i; }
        }
        System.out.println(maxFreq + " " + maxIdx + " " + distinct);
    }
}
