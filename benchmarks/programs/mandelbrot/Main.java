public class Main {
    public static void main(String[] args) {
        int n = Integer.parseInt(args[0]), count = 0;
        for (int y = 0; y < n; y++) for (int x = 0; x < n; x++) {
            double cr = 2.0*x/n - 1.5, ci = 2.0*y/n - 1.0, zr = 0, zi = 0;
            boolean inside = true;
            for (int i = 0; i < 50; i++) {
                double t = zr*zr - zi*zi + cr; zi = 2*zr*zi + ci; zr = t;
                if (zr*zr + zi*zi > 4) { inside = false; break; }
            }
            if (inside) count++;
        }
        System.out.println(count);
    }
}
