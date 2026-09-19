pub const FLASH_BASE: u32 = 0x1000_0000;
pub const FLASH_BYTES: u32 = 2 * 1024 * 1024;
pub const FLASH_END: u32 = FLASH_BASE + FLASH_BYTES;

pub const PAGE_BYTES: u32 = 4096;
pub const WRITE_BYTES: u32 = 1;

pub const BOOTLOADER_BASE: u32 = 0x1000_0000;
pub const BOOTLOADER_BYTES: u32 = 24 * 1024;

pub const ACTIVE_BASE: u32 = 0x1000_6000;
pub const ACTIVE_BYTES: u32 = 780 * 1024;

pub const DFU_BASE: u32 = 0x100C_9000;
pub const DFU_BYTES: u32 = 784 * 1024;

pub const STATE_BASE: u32 = 0x1018_D000;
pub const STATE_BYTES: u32 = 4 * 1024;

pub const SPARE_BASE: u32 = 0x1018_E000;
pub const SPARE_BYTES: u32 = FLASH_END - SPARE_BASE;

const _: () = {
    assert!(BOOTLOADER_BASE.is_multiple_of(PAGE_BYTES));
    assert!(ACTIVE_BASE.is_multiple_of(PAGE_BYTES));
    assert!(DFU_BASE.is_multiple_of(PAGE_BYTES));
    assert!(STATE_BASE.is_multiple_of(PAGE_BYTES));
    assert!(SPARE_BASE.is_multiple_of(PAGE_BYTES));

    assert!(ACTIVE_BYTES.is_multiple_of(PAGE_BYTES));
    assert!(DFU_BYTES.is_multiple_of(PAGE_BYTES));
    assert!(STATE_BYTES.is_multiple_of(PAGE_BYTES));
    assert!(SPARE_BYTES.is_multiple_of(PAGE_BYTES));

    assert!(BOOTLOADER_BASE + BOOTLOADER_BYTES == ACTIVE_BASE);
    assert!(DFU_BASE == ACTIVE_BASE + ACTIVE_BYTES);
    assert!(DFU_BYTES == ACTIVE_BYTES + PAGE_BYTES);
    assert!(STATE_BASE == DFU_BASE + DFU_BYTES);
    assert!(SPARE_BASE == STATE_BASE + STATE_BYTES);
    assert!(SPARE_BASE + SPARE_BYTES == FLASH_END);

    assert!(2 + 4 * (ACTIVE_BYTES / PAGE_BYTES) <= STATE_BYTES / WRITE_BYTES);
};
