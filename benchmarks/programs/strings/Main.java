public class Main {
    public static void main(String[] args) {
        long n = Long.parseLong(args[0]);
        StringBuilder sb = new StringBuilder((int) (n * 6));
        for (long i = 0; i < n; i++) {
            sb.append(i);
            sb.append(' ');
        }
        String result = sb.toString();
        long spaces = 0;
        for (int i = 0; i < result.length(); i++)
            if (result.charAt(i) == ' ') spaces++;
        long h = 0;
        for (int i = 0; i < result.length(); i++)
            h = (h * 31 + result.charAt(i)) & 268435455;
        System.out.println(spaces + " " + h);
    }
}
