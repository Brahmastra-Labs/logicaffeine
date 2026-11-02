#include <stdio.h>
#include <stdlib.h>
long gcd(long a,long b){ if(a<0)a=-a; if(b<0)b=-b; while(b>0){ long t=b; b=a%b; a=t; } return a; }
long lcmf(long a,long b){ if(a==0||b==0) return 0; long g=gcd(a,b); long p=a*b; if(p<0)p=-p; return p/g; }
int main(int argc,char**argv){ long n=atol(argv[1]); long s=0;
	for(long i=1;i<=n;i++) s+=lcmf(i,60); printf("%ld\n",s); return 0; }
