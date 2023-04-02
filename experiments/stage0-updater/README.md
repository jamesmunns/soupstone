# Updating the bootloader

```bash
cd soupstone/firmware/stage0
cargo objcopy \
    --release \
    --features=small \
    -- \
    -O binary \
    ../../experiments/stage0-updater/stage0.bin

cd soupstone/experiments/stage0-updater
cargo objcopy \
    --release \
    -- \
    -O binary \
    ./target/demo.bin

soup-cli stage0 poke \
    -a 0x20000000 \
    -f ./target/demo.bin
soup-cli stage0 bootload \
    -a 0x20000000
```
