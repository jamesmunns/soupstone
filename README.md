# Soupstone

## Current demo step

You need a Seeed XIAO nRF52.

```bash
# Install the Soup CLI
cd host/soup-cli
cargo install -f --path .

# Build the test application
cd ../../experiments/soup-app-demo
cargo objcopy --release \
    -- -O binary ./target/demo.bin

# Place it into RAM
soup-cli stage0 poke \
    -a 0x20000000 \
    -f ./target/demo.bin

# Tell the bootloader to run the app
soup-cli stage0 bootload \
    -a 0x20000000

# Connect to stdin, stderr, and stdout
soup-cli stdio
```

## Doin a release

```bash
# Stage 0 bootloader
cd firmware/stage0
cargo objcopy --release --features=small \
    -- -O binary ./target/stage0.bin

# Factory image
cd ../../experiments/xiao-init-stage-minus-1
cp ../../firmware/stage0/target/stage0.bin .
./generate-uf2.sh

# Make sure the CLI builds
cd ../../host/soup-cli
cargo build --release
```
