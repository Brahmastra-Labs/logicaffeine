/* sw/logos_hal_pci.c against a device tree laid out in files (LOGOS_PCI_DEVICES): open the accelerator, read its ID,
   write and read SCRATCH, ask for a buffer, close it, and report the command word of the function it should have opened
   while open and after close.

     hal_pci_probe <afu id> <function> [buffer bytes] */
#define _GNU_SOURCE
#include "logos_d5005.h"
#include "logos_hal.h"

#include <fcntl.h>
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>

static unsigned command_of(const char *function) {
    char path[4096];
    uint8_t w[2] = {0, 0};
    snprintf(path, sizeof path, "%s/%s/config", getenv("LOGOS_PCI_DEVICES"), function);
    int fd = open(path, O_RDONLY);
    if (fd < 0 || pread(fd, w, 2, 4) != 2) {
        perror(path);
        exit(4);
    }
    close(fd);
    return (unsigned)w[0] | (unsigned)w[1] << 8;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: hal_pci_probe <afu id> <function> [buffer bytes]\n");
        return 4;
    }
    logos_dev *dev = logos_open(argv[1]);
    if (!dev) return 2;
    printf("id %016" PRIx64 "%016" PRIx64 "\n", logos_mmio_read(dev, LOGOS_D5005_REG_ID_H), logos_mmio_read(dev, LOGOS_D5005_REG_ID_L));
    printf("open command 0x%04x\n", command_of(argv[2]));
    logos_mmio_write(dev, LOGOS_D5005_REG_SCRATCH, 0x05ca1ab1e0ddba11ull);
    printf("scratch %016" PRIx64 "\n", logos_mmio_read(dev, LOGOS_D5005_REG_SCRATCH));
    if (argc > 3) {
        uint64_t io = 0;
        void *b = logos_buffer(dev, (size_t)strtoull(argv[3], NULL, 0), &io);
        printf("buffer %s 0x%" PRIx64 "\n", b ? "placed" : "refused", io);
    }
    fflush(stdout);
    logos_close(dev);
    printf("closed command 0x%04x\n", command_of(argv[2]));
    return 0;
}
