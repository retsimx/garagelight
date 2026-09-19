/*
 * ACTIVE application partition (GL-3).
 *
 * FLASH is the ACTIVE slot at 0x10006000 (after the bootloader's BOOT2 +
 * bootloader region). The app must NOT embed boot2, so it links link.x only,
 * never link-rp.x. BOOT2/DFU/STATE exist only to compute the flash-relative
 * __bootloader_* offsets that the embassy-boot FirmwareUpdater reads.
 */
MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    FLASH : ORIGIN = 0x10006000, LENGTH = 780K
    DFU   : ORIGIN = 0x100C9000, LENGTH = 784K
    STATE : ORIGIN = 0x1018D000, LENGTH = 4K
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K
}

__bootloader_dfu_start = ORIGIN(DFU) - ORIGIN(BOOT2);
__bootloader_dfu_end = ORIGIN(DFU) + LENGTH(DFU) - ORIGIN(BOOT2);
__bootloader_state_start = ORIGIN(STATE) - ORIGIN(BOOT2);
__bootloader_state_end = ORIGIN(STATE) + LENGTH(STATE) - ORIGIN(BOOT2);
