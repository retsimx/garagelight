#!/usr/bin/env python3
import struct, sys, hashlib

MAGIC0 = 0x0A324655
MAGIC1 = 0x9E5D5157
MAGICE = 0x0AB16F30
FLAG_FAMILY = 0x00002000
RP2040_FAMILY = 0xE48BFF56
PAYLOAD = 256

# RP2040 bootrom constraints (virtual_disk.c):
#  - every block must have payload_size == 256 (short tail blocks are ignored)
#  - the image MUST be contiguous from the UF2 start address: the bootrom tracks
#    which 4 KiB sectors it has erased via `block_no * 256 / 4096`, so a
#    non-contiguous block numbering writes later sectors without erasing them.
# Therefore build one image from `base` to the highest file end, filling gaps
# with 0x00, and number blocks linearly.


def build(base, pairs):
    # pairs: list of (offset_from_base, path)
    end = 0
    for off, path in pairs:
        end = max(end, off + len(open(path, "rb").read()))
    end = (end + PAYLOAD - 1) // PAYLOAD * PAYLOAD
    image = bytearray(b"\x00" * end)
    for off, path in pairs:
        data = open(path, "rb").read()
        image[off:off + len(data)] = data
    total = end // PAYLOAD
    out = bytearray()
    for n in range(total):
        addr = base + n * PAYLOAD
        blk = bytearray(512)
        struct.pack_into("<IIIIIIII", blk, 0, MAGIC0, MAGIC1, FLAG_FAMILY,
                         addr, PAYLOAD, n, total, RP2040_FAMILY)
        blk[32:32 + PAYLOAD] = image[n * PAYLOAD:(n + 1) * PAYLOAD]
        struct.pack_into("<I", blk, 508, MAGICE)
        out += blk
    return bytes(out), total, end


def main():
    outpath = sys.argv[1]
    base = int(sys.argv[2], 0)
    pairs = []
    args = sys.argv[3:]
    for i in range(0, len(args), 2):
        pairs.append((int(args[i], 0), args[i + 1]))
    data, total, end = build(base, pairs)
    open(outpath, "wb").write(data)
    print(f"{outpath}: base={hex(base)} end={hex(base+end)} {len(data)} bytes, {total} blocks, sha256={hashlib.sha256(data).hexdigest()}")


if __name__ == "__main__":
    main()
