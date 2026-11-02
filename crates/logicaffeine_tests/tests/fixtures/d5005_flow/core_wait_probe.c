/* logos_core_run (sw/logos_core.h) against a scripted device, its poll budget 64 register reads: each STATUS_SEQ read
   is a tick of the device's clock. One command, tag 0x2a, is rung; the scenario says what the device does while it
   runs, and the probe prints the record the wait returned and the ticks it took.

     core_wait_probe moving|stopped|other|late|unsequenced|silent

     moving   a progress record of the command every 40 ticks, its progress one more each, its own record at tick 1000
     stopped  progress records every 40 ticks until tick 300, then none, and no record of its own
     other    progress records every 40 ticks, moving, of another command's tag, and no record of its own
     late     a progress record at tick 40, then its own record just as the wait, its budget spent, reads the status
     unsequenced  records of the command every 40 ticks, its code not RUNNING, their result0 moving, the count of
              completed commands never moving
     silent   no record at all
   A wait that has not ended by tick 100000 ends the probe with status 5. */
#include "logos_core.h"

#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct logos_dev {
    const char *scenario;
    uint64_t ticks, status, result0, result1, seq;
};

static const uint64_t TAG = 0x2a;

static void complete(logos_dev *d) {
    d->status = TAG << 8 | LOGOS_D5005_STATUS_OK;
    d->result0 = 7;
    d->result1 = 123456;
    d->seq = 1;
}

static void advance(logos_dev *d) {
    uint64_t t = ++d->ticks;
    if (t > 100000) {
        printf("the wait never ended\n");
        exit(5);
    }
    int moving = strcmp(d->scenario, "moving") == 0, stopped = strcmp(d->scenario, "stopped") == 0;
    int other = strcmp(d->scenario, "other") == 0, late = strcmp(d->scenario, "late") == 0;
    int unsequenced = strcmp(d->scenario, "unsequenced") == 0;
    if ((moving || stopped || other || late || unsequenced) && t % 40 == 0 && (!stopped || t <= 300) && (!late || t == 40)) {
        d->status = (other ? TAG + 1 : TAG) << 8 | (unsequenced ? LOGOS_D5005_STATUS_OK : LOGOS_D5005_STATUS_RUNNING);
        d->result0 = t / 40;
        d->result1 = t * 1000;
    }
    if (moving && t == 1000) complete(d);
}

logos_dev *logos_open(const char *afu_id) {
    logos_dev *d = calloc(1, sizeof *d);
    d->scenario = afu_id;
    return d;
}

void logos_close(logos_dev *dev) { free(dev); }

uint64_t logos_mmio_read(logos_dev *dev, uint32_t byte_offset) {
    switch (byte_offset) {
    case LOGOS_D5005_REG_STATUS_SEQ: advance(dev); return dev->seq;
    case LOGOS_D5005_REG_CMD_SPACE: return 16;
    case LOGOS_D5005_REG_STATUS:
        if (strcmp(dev->scenario, "late") == 0 && dev->ticks > 60 && dev->seq == 0) complete(dev);
        return dev->status;
    case LOGOS_D5005_REG_RESULT0: return dev->result0;
    case LOGOS_D5005_REG_RESULT1: return dev->result1;
    default: return 0;
    }
}

void logos_mmio_write(logos_dev *dev, uint32_t byte_offset, uint64_t value) {
    (void)dev;
    (void)byte_offset;
    (void)value;
}

void *logos_buffer(logos_dev *dev, size_t bytes, uint64_t *io_address) {
    (void)dev;
    (void)bytes;
    *io_address = 0;
    return NULL;
}

void logos_buffer_free(logos_dev *dev, void *buffer) {
    (void)dev;
    (void)buffer;
}

double logos_user_clock_hz(logos_dev *dev) {
    (void)dev;
    return 0;
}

int main(int argc, char **argv) {
    if (argc != 2) {
        fprintf(stderr, "usage: core_wait_probe moving|stopped|other|late|unsequenced|silent\n");
        return 2;
    }
    logos_dev *dev = logos_open(argv[1]);
    logos_record r = logos_core_run(dev, LOGOS_D5005_OP_FILL, TAG, 1, 0, 64);
    printf("status 0x%" PRIx64 " result0 %" PRIu64 " result1 %" PRIu64 " ticks %" PRIu64 "\n", r.status, r.result0,
           r.result1, dev->ticks);
    logos_close(dev);
    return 0;
}
