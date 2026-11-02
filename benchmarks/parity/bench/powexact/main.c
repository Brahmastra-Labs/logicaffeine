#include <stdio.h>
#include <stdlib.h>
long powexact(long base, long exp){
    long acc=1, b=base, e=exp;
    while(e>0){ if(e%2==1) acc=acc*b; e/=2; b=b*b; }
    return acc;
}
int main(int argc,char**argv){
    long n=atol(argv[1]); long s=0;
    for(long i=1;i<=n;i++) s+=powexact((i%5)+1, i%13);
    printf("%ld\n", s); return 0;
}
