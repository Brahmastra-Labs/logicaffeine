#include <stdio.h>
#include <stdlib.h>

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    long n = atol(argv[1]);
    double sum = 0.0;
    double sign = 1.0;
    for (long k = 0; k < n; k++) {
        sum += sign / (2.0 * k + 1.0);
        sign = -sign;
    }
    printf("%.15f\n", sum * 4.0);
    return 0;
}
