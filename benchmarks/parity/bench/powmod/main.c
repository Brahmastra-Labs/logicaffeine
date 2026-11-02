#include <stdio.h>
#include <stdlib.h>
long powmod(long base,long exp,long m){ long acc=1, b=base%m, e=exp;
	while(e>0){ if(e%2==1) acc=(acc*b)%m; e/=2; b=(b*b)%m; } return acc; }
int main(int argc,char**argv){ long n=atol(argv[1]); long m=1000000007; long s=0;
	for(long i=1;i<=n;i++) s+=powmod(i,i,m); printf("%ld\n",s); return 0; }
