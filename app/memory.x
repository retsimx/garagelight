/*
 * ACTIVE application partition (GL-1).
 *
 * The ACTIVE slot starts at 0x10006000 (after the GL-3 bootloader's BOOT2 +
 * metadata region) and is 780K long. The app must NOT embed boot2: it is
 * deliberately linked with `link.x` + `defmt.x` only, never `link-rp.x`.
 */
MEMORY {
    FLASH : ORIGIN = 0x10006000, LENGTH = 780K
    RAM   : ORIGIN = 0x20000000, LENGTH = 264K
}
