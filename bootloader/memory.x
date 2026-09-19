/*
 * Bootloader memory map and the pinned GL-3 partition table
 * (retsimx/garagelight#4). The bootloader links link-rp.x and owns boot2.
 *
 * The __bootloader_* symbols are flash-relative: embassy-boot maps the active,
 * dfu and state partitions as offsets from the start of flash, so each symbol
 * subtracts ORIGIN(BOOT2) rather than carrying an absolute address.
 */
MEMORY {
    BOOT2  : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH  : ORIGIN = 0x10000100, LENGTH = 24K - 0x100
    ACTIVE : ORIGIN = 0x10006000, LENGTH = 780K
    DFU    : ORIGIN = 0x100C9000, LENGTH = 784K
    STATE  : ORIGIN = 0x1018D000, LENGTH = 4K
    RAM    : ORIGIN = 0x20000000, LENGTH = 264K
}

__bootloader_state_start = ORIGIN(STATE) - ORIGIN(BOOT2);
__bootloader_state_end = ORIGIN(STATE) + LENGTH(STATE) - ORIGIN(BOOT2);
__bootloader_active_start = ORIGIN(ACTIVE) - ORIGIN(BOOT2);
__bootloader_active_end = ORIGIN(ACTIVE) + LENGTH(ACTIVE) - ORIGIN(BOOT2);
__bootloader_dfu_start = ORIGIN(DFU) - ORIGIN(BOOT2);
__bootloader_dfu_end = ORIGIN(DFU) + LENGTH(DFU) - ORIGIN(BOOT2);
