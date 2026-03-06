public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        char[] text = new char[n];
        int pos = 0;
        while (pos < n) {
            if (pos > 0 && pos % 1000 == 0 && pos + 5 <= n) {
                text[pos]='X'; text[pos+1]='X'; text[pos+2]='X'; text[pos+3]='X'; text[pos+4]='X';
                pos += 5;
            } else {
                text[pos] = (char)('a' + pos % 5);
                pos++;
            }
        }
        String needle = "XXXXX";
        int needleLen = 5;
        long count = 0;
        for (int i = 0; i <= n - needleLen; i++) {
            boolean match = true;
            for (int j = 0; j < needleLen; j++) {
                if (text[i + j] != needle.charAt(j)) { match = false; break; }
            }
            if (match) count++;
        }
        System.out.println(count);
    }
}
