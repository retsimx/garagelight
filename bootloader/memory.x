/*
 * PLACEHOLDER memory map for the bootloader crate.
 *
 * GL-3 (retsimx/garagelight#4) owns the real bootloader memory map and the
 * partition table. The values below are a minimal stand-in so the workspace
 * links; they reserve a 256-byte BOOT2 region and a 24K FLASH region at the
 * start of flash. GL-3 will replace this file with the real layout.
 */
MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10000100, LENGTH = 24K - 0x100
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K
}
