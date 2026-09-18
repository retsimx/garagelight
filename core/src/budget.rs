pub const ACTIVE_BASE: u32 = 0x1000_6000;
pub const ACTIVE_FLASH_BYTES: u32 = 780 * 1024;
pub const BLOB_TOTAL_BYTES: u32 = 238_967;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn fits(used_bytes: u32) -> bool {
    used_bytes <= ACTIVE_FLASH_BYTES
}
