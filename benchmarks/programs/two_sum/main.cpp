#include <cstdio>
#include <cstdlib>
#include <unordered_set>
int main(int argc, char *argv[]) {
    if (argc<2) return 1;
    long n = atol(argv[1]);
    long *arr = new long[n];
    long seed = 42;
    for (long i=0;i<n;i++) { seed=(seed*1103515245+12345)%2147483648L; arr[i]=((seed>>16)&0x7fff)%n; }
    std::unordered_set<long> seen;
    long count = 0;
    for (long i=0;i<n;i++) {
        long c = n - arr[i];
        if (c >= 0 && seen.count(c)) count++;
        seen.insert(arr[i]);
    }
    printf("%ld\n", count);
    delete[] arr;
    return 0;
}
