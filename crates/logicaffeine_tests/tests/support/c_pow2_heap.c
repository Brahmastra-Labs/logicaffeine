#include <malloc.h>
int main(int argc, char **argv) {
    int n=@TURNS@;
    size_t before=mallinfo2().uordblks;
    LInt squared=lint_mul(lint_small(4294967296LL),lint_small(4294967296LL));
    LInt input=lint_sub(squared,lint_small(1));
    if(squared.big){lbig_free(squared.big);lc_gc_free(squared.big);}
    int64_t value=0;
    for(int i=0;i<n;i++) value=logos_lint_mod_pow2_nonnegative(input,4611686018427387904LL);
    if(input.big){lbig_free(input.big);lc_gc_free(input.big);}
    size_t after=mallinfo2().uordblks;
    printf("%lld\nretained=%zu\n",(long long)value,after>=before?after-before:0);
}
