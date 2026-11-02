#include <stdio.h>
#include <stdlib.h>
int main(int argc, char** argv){
    long n = atol(argv[1]);
    long long sum = 0;
    for (long i = 1; i <= n; i++){ long x = i - 10000000; sum += (x>0) - (x<0); }
    printf("%lld\n", sum);
    return 0;
}
