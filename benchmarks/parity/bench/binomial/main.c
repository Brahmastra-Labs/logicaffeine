#include <stdio.h>
#include <stdlib.h>
long binom(long n,long k){ if(k<0) return 0; if(k>n) return 0; long kk=k; if(kk>n-kk) kk=n-kk;
	long acc=1; for(long i=1;i<=kk;i++) acc=acc*(n-kk+i)/i; return acc; }
int main(int argc,char**argv){ long n=atol(argv[1]); long s=0;
	for(long i=1;i<=n;i++) s+=binom(30, (s+i)%16); printf("%ld\n",s); return 0; }
