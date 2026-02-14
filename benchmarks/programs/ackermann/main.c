#include <stdio.h>
#include <stdlib.h>

long ackermann(long m, long n) {
    if (m == 0) return n + 1;
    if (n == 0) return ackermann(m - 1, 1);
    return ackermann(m - 1, ackermann(m, n - 1));
}

int main(int argc, char *argv[]) {
    if (argc < 2) { fprintf(stderr, "Usage: ackermann <m>\n"); return 1; }
    long m_val = atol(argv[1]);
    printf("%ld\n", ackermann(3, m_val));
    return 0;
}
