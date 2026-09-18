#!/usr/bin/env python3
"""Build a patched stock boot2 shim that jumps to a no-boot2 app.

GL-1/GL-2 bring-up workaround: the ACTIVE app links at 0x10006000 with no
boot2 of its own, and probe-rs run resets the chip to the reset vector, so a
shim at 0x10000000 is needed to jump into the app. GL-3 replaces this with the
real bootloader.

Patches the 256-byte boot2_w25q080.padded.bin from the rp2040-boot2 crate:
  - offset 232: entry literal (little-endian u32)
  - offset 252: CRC-32/MPEG-2 over the first 252 bytes (little-endian u32)
"""

import argparse
import glob
import os
import struct
import sys

DEFAULT_GLOB = "~/.cargo/registry/src/*/rp2040-boot2-*/bin/boot2_w25q080.padded.bin"
ENTRY_OFFSET = 232
CRC_OFFSET = 252
CRC_LEN = 252


def crc32_mpeg2(data):
    crc = 0xFFFFFFFF
    for byte in data:
        crc ^= byte << 24
        for _ in range(8):
            if crc & 0x80000000:
                crc = ((crc << 1) ^ 0x04C11DB7) & 0xFFFFFFFF
            else:
                crc = (crc << 1) & 0xFFFFFFFF
    return crc


def find_input():
    matches = sorted(glob.glob(os.path.expanduser(DEFAULT_GLOB)))
    if not matches:
        sys.exit("error: no rp2040-boot2 padded bin found; pass --input")
    return matches[-1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", help="source boot2 padded bin")
    parser.add_argument(
        "--entry", type=lambda v: int(v, 0), default=0x10006000,
        help="app entry address (default 0x10006000)",
    )
    parser.add_argument("--output", default="boot2_shim.bin", help="output path")
    args = parser.parse_args()

    src = args.input or find_input()
    with open(src, "rb") as f:
        image = bytearray(f.read())
    if len(image) != 256:
        sys.exit(f"error: expected a 256-byte boot2 image, got {len(image)} from {src}")

    struct.pack_into("<I", image, ENTRY_OFFSET, args.entry)
    crc = crc32_mpeg2(image[:CRC_LEN])
    struct.pack_into("<I", image, CRC_OFFSET, crc)

    with open(args.output, "wb") as f:
        f.write(image)

    print(f"input:  {src}")
    print(f"output: {args.output}")
    print(f"entry:  {args.entry:#010x}")
    print(f"crc:    {crc:#010x}")


if __name__ == "__main__":
    main()
