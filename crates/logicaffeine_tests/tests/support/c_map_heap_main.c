int main(int argc, char **argv) {
    int status = map_program_main(argc, argv);
    size_t during = lc_gc_collections, high = lc_gc_high;
    size_t physical = (size_t)mallinfo2().uordblks;
    if (physical > map_heap_peak) map_heap_peak = physical;
    lc_gc_collect();
    size_t retained = (size_t)mallinfo2().uordblks;
    fprintf(stderr, "%zu %zu %zu %zu\n", retained, map_heap_peak, during, high);
    return status;
}
