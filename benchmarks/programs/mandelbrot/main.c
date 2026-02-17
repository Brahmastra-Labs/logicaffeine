#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    int count = 0;
    for (int y = 0; y < n; y++) {
        for (int x = 0; x < n; x++) {
            double cr = 2.0 * x / n - 1.5;
            double ci = 2.0 * y / n - 1.0;
            double zr = 0.0, zi = 0.0;
            int inside = 1;
            for (int iter = 0; iter < 50; iter++) {
                double zr2 = zr * zr - zi * zi + cr;
                double zi2 = 2.0 * zr * zi + ci;
                zr = zr2;
                zi = zi2;
                if (zr * zr + zi * zi > 4.0) { inside = 0; break; }
            }
            if (inside) count++;
        }
    }
    printf("%d\n", count);
    return 0;
}
