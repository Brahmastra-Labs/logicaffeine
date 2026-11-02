#include <malloc.h>
#include <stdio.h>
static size_t map_heap_samples, map_heap_peak;
static void map_heap_sample(void) {
    if ((++map_heap_samples & 255) != 0) return;
    size_t physical = (size_t)mallinfo2().uordblks;
    if (physical > map_heap_peak) map_heap_peak = physical;
}
