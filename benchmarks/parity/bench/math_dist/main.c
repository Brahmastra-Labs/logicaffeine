#include <stdio.h>
#include <stdlib.h>
#include <math.h>
int main(int argc,char**argv){if(argc<2)return 1;long n=atol(argv[1]);long s=0;
for(long i=1;i<=n;i++){double dx=(double)i/1000.0-3.0,dy=(double)i/700.0-4.0;s+=(long)(sqrt(dx*dx+dy*dy)*1000000.0);}
printf("%ld\n",s);return 0;}
