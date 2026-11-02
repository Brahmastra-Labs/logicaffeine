#include <stdio.h>
#include <stdlib.h>
int main(int argc, char** argv){
    long n = atol(argv[1]);
    long long sum = 0;
    for (long i = 1; i <= n; i++){ long a = i, b = n - i; sum += (a>b?a:b); }
    printf("%lld\n", sum);
    return 0;
}
