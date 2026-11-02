/* Reset an OFS port's accelerator through its DFL port device: the port held in reset and released, as the PR flow
   leaves it, so every register of the accelerator starts from its reset value at the user clock now set.
     logos_port_reset <port device, e.g. /dev/dfl-port.0> */
#include <fcntl.h>
#include <linux/fpga-dfl.h>
#include <stdio.h>
#include <sys/ioctl.h>
#include <unistd.h>

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: logos_port_reset <port device>\n");
        return 2;
    }
    int fd = open(argv[1], O_RDWR);
    if (fd < 0) {
        perror(argv[1]);
        return 2;
    }
    if (ioctl(fd, DFL_FPGA_PORT_RESET) != 0) {
        perror("DFL_FPGA_PORT_RESET");
        close(fd);
        return 1;
    }
    close(fd);
    fprintf(stderr, "logos_port_reset: %s reset\n", argv[1]);
    return 0;
}
