/* The physical backend's frame walk (`frames` in sw/logos_hal_pci.c) over a page map laid out in a file: the address it
   finds for `bytes` at virtual address `address`, or its refusal.

     hal_pci_frames_probe <page map file> <address> <bytes> */
#include "logos_hal_pci.c"

#include <inttypes.h>

int main(int argc, char **argv) {
    if (argc != 4) {
        fprintf(stderr, "usage: hal_pci_frames_probe <page map file> <address> <bytes>\n");
        return 4;
    }
    int fd = open(argv[1], O_RDONLY);
    if (fd < 0) {
        perror(argv[1]);
        return 4;
    }
    uint64_t found = 0;
    int r = frames(fd, (const void *)(uintptr_t)strtoull(argv[2], NULL, 0), (size_t)strtoull(argv[3], NULL, 0), &found);
    close(fd);
    if (r == 0) printf("placed 0x%" PRIx64 "\n", found);
    else printf("refused\n");
    return 0;
}
