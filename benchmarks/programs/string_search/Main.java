public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        byte[] text = new byte[n];
        long seed = 42;
        for (int pos = 0; pos < n; pos++) {
            seed = (seed * 1103515245 + 12345) % 2147483648L;
            text[pos] = (byte) ('a' + (seed >> 16) % 5);
        }
        byte[] needle = "cabed".getBytes(java.nio.charset.StandardCharsets.US_ASCII);
        int needleLen = 5;
        long count = 0;
        long[] positions = new long[1024];
        for (int i = 0; i <= n - needleLen; i++) {
            boolean match = true;
            for (int j = 0; j < needleLen; j++) {
                if (text[i + j] != needle[j]) { match = false; break; }
            }
            if (match) {
                if (count == positions.length) positions = java.util.Arrays.copyOf(positions, positions.length * 2);
                positions[(int) count++] = i;
            }
        }
        long checksum = 0;
        for (int k = 0; k < count; k++) checksum = (checksum * 31 + positions[k]) & 268435455;
        System.out.println(count + " " + checksum);
    }
}
