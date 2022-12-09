rustup target add thumbv6m-none-eabi
cargo install uf2conv cargo-binutils
rustup component add llvm-tools-preview

# Put WIFI_NETWORK and WIFI_PASSWORD in this file
source .env

mkdir -p build
cargo build --release
cargo objcopy --release -- -O binary build/garagelight.bin
uf2conv build/garagelight.bin --base 0x10000000 --family 0xe48bff56 --output build/garagelight.uf2
