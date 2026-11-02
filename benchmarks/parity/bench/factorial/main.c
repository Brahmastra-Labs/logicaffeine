#include <stdio.h>
#include <stdlib.h>
long fact(long n){ long r=1; for(long i=2;i<=n;i++) r*=i; return r; }
int main(int argc,char**argv){ long n=atol(argv[1]); long s=0;
	for(long i=1;i<=n;i++) s+=fact((i%12)+1); printf("%ld\n",s); return 0; }
