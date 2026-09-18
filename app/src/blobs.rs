//! CYW43 firmware blobs, aligned to A4 so the cyw43 driver can hand them to the
//! chip without a bounce buffer.
//!
//! Eventual GL-2 calls (recorded here so the blobs are never reshaped):
//!   `cyw43::new_with_bluetooth(state, pwr, spi, WIFI_FW, BT_FW, NVRAM)`
//!   `control.init(CLM)`

pub static WIFI_FW: &cyw43::Aligned<cyw43::A4, [u8]> =
    cyw43::aligned_bytes!("../cyw43-firmware/43439A0.bin");
pub static BT_FW: &cyw43::Aligned<cyw43::A4, [u8]> =
    cyw43::aligned_bytes!("../cyw43-firmware/43439A0_btfw.bin");
pub static NVRAM: &cyw43::Aligned<cyw43::A4, [u8]> =
    cyw43::aligned_bytes!("../cyw43-firmware/nvram_rp2040.bin");
pub static CLM: &cyw43::Aligned<cyw43::A4, [u8]> =
    cyw43::aligned_bytes!("../cyw43-firmware/43439A0_clm.bin");
