public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]);
        StringBuilder sb = new StringBuilder(n * 6);
        for (int i = 0; i < n; i++) {
            sb.append(i);
            sb.append(' ');
        }
        String result = sb.toString();
        int spaces = 0;
        for (int i = 0; i < result.length(); i++)
            if (result.charAt(i) == ' ') spaces++;
        System.out.println(spaces);
    }
}
