
static size_t trace_root(const void *root, size_t size) {
    lc_gc_prepare(); lc_gc_pending_n=0;
    lc_gc_scan((uintptr_t)root,(uintptr_t)root+size);
    while(lc_gc_pending_n) {
        lc_gc_range r=lc_gc_pending[--lc_gc_pending_n];
        lc_gc_scan(r.lo,r.hi);
    }
    size_t freed=0, live=lc_gc_sweep(&freed);
    lc_gc_live=live;
    return live;
}
int main(int argc,char **argv) {
    int mode=atoi(argv[1]);
    size_t expected=0,actual=0;
    if(mode==0) {
        Map_str_str map=map_str_str_new(); expected=lc_gc_live;
        char *key=lc_gc_strdup("key"),*value=lc_gc_strdup("payload");
        map_str_str_set(&map,key,value); map_str_str_remove(&map,key);
        if(map.len || map.ord_len) return 8;
        actual=trace_root(&map,sizeof map);
    } else if(mode==1) {
        Map_i64_i64 map=map_i64_i64_new(); expected=lc_gc_live;
        int64_t *box=lc_gc_alloc(8);*box=42;
        map_i64_i64_set(&map,1,(int64_t)(intptr_t)box|1);map_i64_i64_remove(&map,1);
        if(map.len || map.ord_len) return 8;
        actual=trace_root(&map,sizeof map);
    } else if(mode==2) {
        Seq_str seq=seq_str_with_capacity(8); expected=lc_gc_live;
        char *value=lc_gc_strdup("payload");seq_str_push(&seq,value);
        if(seq_str_pop(&seq)!=value || seq.len) return 8;
        actual=trace_root(&seq,sizeof seq);
    } else if(mode==3) {
        Seq_i64 seq=seq_i64_with_capacity(8); expected=lc_gc_live;
        int64_t *box=lc_gc_alloc(8);*box=42;int64_t v=(int64_t)(intptr_t)box|1;
        seq_i64_push(&seq,v);if(seq_i64_pop(&seq)!=v || seq.len) return 8;
        actual=trace_root(&seq,sizeof seq);
    } else if(mode==4) {
        Set_str set=set_str_new();expected=lc_gc_live;
        char *key=lc_gc_strdup("key");set_str_add(&set,key);set_str_remove(&set,key);
        if(set.len || set.ord_len) return 8;
        actual=trace_root(&set,sizeof set);
    } else if(mode==5) {
        Map_i64_str map=map_i64_str_new();expected=lc_gc_live;
        char *value=lc_gc_strdup("payload");map_i64_str_set(&map,1,value);map_i64_str_remove(&map,1);
        if(map.len || map.ord_len) return 8;
        actual=trace_root(&map,sizeof map);
    } else if(mode==6) {
        Map_str_i64 map=map_str_i64_new();expected=lc_gc_live;
        char *key=lc_gc_strdup("key");int64_t *box=lc_gc_alloc(8);*box=42;
        map_str_i64_set(&map,key,(int64_t)(intptr_t)box|1);map_str_i64_remove(&map,key);
        if(map.len || map.ord_len) return 8;
        actual=trace_root(&map,sizeof map);
    } else return 9;
    printf("mode=%d expected=%zu actual=%zu\n",mode,expected,actual);
    return actual==expected?0:7;
}
