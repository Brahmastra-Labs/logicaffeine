// analytics — filter → map → group-by → reduce pipeline (Java port of main.c).
public class Main {
    static final long MOD = 1000000007L;
    static final int NGROUP = 32768;

    public static void main(String[] args) {
        if (args.length < 1) {
            System.err.println("usage: analytics <n>");
            System.exit(1);
        }
        long n = Long.parseLong(args[0]);
        long seed = 42;
        long[] groups = new long[NGROUP];
        long kept = 0;
        for (long r = 0; r < n; r++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            long key = (seed >> 16) & 0x7fff;
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            long value = (seed >> 16) & 0x7fff;
            if (value % 3 == 0) continue;                  // FILTER
            long mapped = (value * key + 7) % MOD;          // MAP
            int g = (int) (key % NGROUP);                   // GROUP BY key
            groups[g] = (groups[g] + mapped) % MOD;         // aggregate SUM
            kept++;
        }
        long checksum = 0;                                  // REDUCE
        for (int g = 0; g < NGROUP; g++) {
            checksum = (checksum * 31 + groups[g]) & 268435455;
        }
        System.out.printf("%d%n%d%n", kept, checksum);
    }
}
