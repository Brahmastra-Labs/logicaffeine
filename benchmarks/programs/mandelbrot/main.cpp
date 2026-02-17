#include <cstdio>
#include <cstdlib>
int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    int count = 0;
    for (int y = 0; y < n; y++) for (int x = 0; x < n; x++) {
        double cr = 2.0*x/n - 1.5, ci = 2.0*y/n - 1.0, zr = 0, zi = 0;
        int inside = 1;
        for (int i = 0; i < 50; i++) {
            double t = zr*zr - zi*zi + cr; zi = 2*zr*zi + ci; zr = t;
            if (zr*zr + zi*zi > 4) { inside = 0; break; }
        }
        if (inside) count++;
    }
    printf("%d\n", count);
    return 0;
}
