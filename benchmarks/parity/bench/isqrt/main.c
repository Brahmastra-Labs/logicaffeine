#include <stdio.h>
#include <stdlib.h>
long isqrtf(long n){ if(n<2) return n; long x=n, y=(x+1)/2; while(y<x){ x=y; y=(x+n/x)/2; } return x; }
int main(int argc,char**argv){ long n=atol(argv[1]); long s=0;
	for(long i=1;i<=n;i++) s+=isqrtf(i); printf("%ld\n",s); return 0; }
