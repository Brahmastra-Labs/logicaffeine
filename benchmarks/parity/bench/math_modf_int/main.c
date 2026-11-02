#include <stdio.h>
#include <stdlib.h>
#include <math.h>
int main(int argc,char**argv){if(argc<2)return 1;long n=atol(argv[1]);long s=0;
for(long i=1;i<=n;i++){double ip;modf((double)i/1000.0,&ip);s+=(long)ip;}
printf("%ld\n",s);return 0;}
