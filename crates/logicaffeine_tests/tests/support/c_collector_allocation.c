
int main(int argc,char **argv) {
    int mode=atoi(argv[1]);
    if(mode==0) { void *p=lc_gc_alloc(SIZE_MAX); printf("oversize allocation returned %d\n",p!=NULL); return 7; }
    if(mode==1) { void *p=lc_gc_alloc(8); p=lc_gc_realloc(p,SIZE_MAX); printf("oversize resize returned %d\n",p!=NULL); return 7; }
    if(mode==2) { void *p=lc_gc_calloc(SIZE_MAX,2); printf("oversize product returned %d\n",p!=NULL); return 7; }
    if(mode==3) {
        size_t n=8192;
        void *p=lc_gc_alloc(n); memset(p,0x7f,n); lc_gc_free(p);
        unsigned char *q=lc_gc_alloc(n);
        size_t dirty=0; for(size_t i=0;i<n;i++) dirty+=q[i]!=0;
        printf("uninitialized traced bytes=%zu\n",dirty);
        return dirty?7:0;
    }
    if(mode==4) {
        void *p=lc_gc_alloc(9); size_t capacity=lc_gc_capacity(p);
        printf("checked allocation capacity=%zu\n",capacity);
        return capacity==9?0:7;
    }
    return 8;
}
