#include <stdio.h>
#include <stdlib.h>
int main(int argc, char** argv){
    long n = atol(argv[1]);
    long long sum = 0;
    for (long i = 1; i <= n; i++){ long v = i; if (v < 5000000) v = 5000000; if (v > 15000000) v = 15000000; sum += v; }
    printf("%lld\n", sum);
    return 0;
}
