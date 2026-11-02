#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
int main(int argc,char**argv){if(argc<2)return 1;long n=atol(argv[1]);long s=0;
for(long i=1;i<=n;i++){uint32_t x=(uint32_t)i;s+=(long)((x<<7)|(x>>25));}
printf("%ld\n",s);return 0;}
