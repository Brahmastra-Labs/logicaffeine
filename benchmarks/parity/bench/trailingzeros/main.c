#include <stdio.h>
#include <stdlib.h>
int main(int argc,char**argv){ long n=atol(argv[1]); long s=0;
	for(long i=1;i<=n;i++) s+=__builtin_ctzll((unsigned long long)i);
	printf("%ld\n",s); return 0; }
