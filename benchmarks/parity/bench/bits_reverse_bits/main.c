#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
int main(int argc,char**argv){if(argc<2)return 1;long n=atol(argv[1]);long s=0;
for(long i=1;i<=n;i++){uint32_t x=(uint32_t)i,r=0;for(int b=0;b<32;b++){r=(r<<1)|(x&1);x>>=1;}s+=(long)r;}
printf("%ld\n",s);return 0;}
