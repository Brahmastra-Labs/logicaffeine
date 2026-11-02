#include <malloc.h>
#include <assert.h>
static void release_exact_result(int64_t value) {
    if (value & 1) {
        LInt *box = (LInt *)(intptr_t)(value ^ 1);
        if (box->big) { lbig_free(box->big); lc_gc_free(box->big); }
        lc_gc_free(box);
    }
}
int main(int argc, char **argv) {
    int turns = @TURNS@;
    int large_divisor = @LARGE_DIVISOR@;
    int64_t left = lc_xof(-5000000000000000000LL);
    int64_t right = lc_xof(5000000000000000000LL);
    int64_t dividend = lc_xmul(left, right);
    release_exact_result(left);
    release_exact_result(right);
    int64_t factor = lc_xof(4000000000000000000LL);
    int64_t divisor = large_divisor ? lc_xmul(factor, factor) : lc_xof(3);
    release_exact_result(factor);
    size_t before = mallinfo2().uordblks;
    for (int i=0; i<turns; i++) {
        int64_t answer = lc_xfloordiv(dividend, divisor);
        if (i == turns-1) {
            char *text = lint_to_string(lc_xget(answer));
            puts(text);
            lc_gc_free(text);
        }
        release_exact_result(answer);
    }
    size_t after = mallinfo2().uordblks;
    printf("retained=%zu\n", after > before ? after-before : 0);
    release_exact_result(dividend);
    release_exact_result(divisor);
    return 0;
}
