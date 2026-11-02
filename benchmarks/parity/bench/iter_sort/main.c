#include <stdio.h>
#include <stdlib.h>
#include <string.h>
static int cmpl(const void*a,const void*b){long x=*(const long*)a,y=*(const long*)b;return (x>y)-(x<y);}
int main(int argc, char** argv){
    long reps = atol(argv[1]);
    long M = 2048;
    long* xs = malloc(M * sizeof(long));
    for (long k=0;k<M;k++) xs[k]=(k*2654435761L) % 1048576L;
    long checksum = 0;
    for (long r=0;r<reps;r++){
        xs[r % M] = r % 1048576L;
        long* res=malloc(M*sizeof(long)); memcpy(res,xs,M*sizeof(long)); qsort(res,M,sizeof(long),cmpl); checksum+=res[r % M]; free(res);
    }
    printf("%ld\n", checksum);
    return 0;
}
