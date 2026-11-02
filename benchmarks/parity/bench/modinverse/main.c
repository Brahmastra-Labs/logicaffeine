#include <stdio.h>
#include <stdlib.h>
long modinverse(long x, long m){
    long oldR=x%m; if(oldR<0) oldR+=m;
    long r=m, oldS=1, s=0;
    while(r>0){
        long q=oldR/r;
        long tR=oldR-q*r; oldR=r; r=tR;
        long tS=oldS-q*s; oldS=s; s=tS;
    }
    if(oldR>1) return 0;
    long inv=oldS%m;
    if(inv<0) return inv+m;
    return inv;
}
int main(int argc,char**argv){
    long n=atol(argv[1]); long m=1000000007; long sum=0;
    for(long i=1;i<=n;i++) sum+=modinverse(i,m);
    printf("%ld\n", sum); return 0;
}
