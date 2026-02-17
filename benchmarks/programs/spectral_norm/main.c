#include <stdio.h>
#include <stdlib.h>
#include <math.h>

double A(int i, int j) {
    return 1.0 / ((double)((i + j) * (i + j + 1) / 2 + i + 1));
}

void mul_Av(int n, double *v, double *Av) {
    for (int i = 0; i < n; i++) {
        Av[i] = 0.0;
        for (int j = 0; j < n; j++) Av[i] += A(i, j) * v[j];
    }
}

void mul_Atv(int n, double *v, double *Atv) {
    for (int i = 0; i < n; i++) {
        Atv[i] = 0.0;
        for (int j = 0; j < n; j++) Atv[i] += A(j, i) * v[j];
    }
}

void mul_AtAv(int n, double *v, double *AtAv, double *tmp) {
    mul_Av(n, v, tmp);
    mul_Atv(n, tmp, AtAv);
}

int main(int argc, char *argv[]) {
    if (argc < 2) return 1;
    int n = atoi(argv[1]);
    double *u = malloc(n * sizeof(double));
    double *v = malloc(n * sizeof(double));
    double *tmp = malloc(n * sizeof(double));
    for (int i = 0; i < n; i++) u[i] = 1.0;
    for (int i = 0; i < 10; i++) {
        mul_AtAv(n, u, v, tmp);
        mul_AtAv(n, v, u, tmp);
    }
    double vBv = 0.0, vv = 0.0;
    for (int i = 0; i < n; i++) { vBv += u[i] * v[i]; vv += v[i] * v[i]; }
    printf("%.9f\n", sqrt(vBv / vv));
    free(u); free(v); free(tmp);
    return 0;
}
