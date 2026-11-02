#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
int main(int argc,char**argv){if(argc<2)return 1;long n=atol(argv[1]);long s=0;
for(long i=1;i<=n;i++){s+=(long)__builtin_bswap32((uint32_t)i);}
printf("%ld\n",s);return 0;}
