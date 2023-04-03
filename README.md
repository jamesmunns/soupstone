# Soupstone

## Current demo step

You need a Seeed XIAO nRF52. It needs to have the stage0 bootloader on it.

### Flashing the soupstone bootloader

> NOTE: This removes the existing bootloader! This can be undone, but there
> is no documentation on how to do this yet. You may need to use an SWD
> debugger to replace the original bootloader!

If you have a XIAO nRF52 fresh out of the box, grab the `minus-1.uf2` file from
the [releases] page, and copy it to the flash drive that appears when you plug
in your XIAO. It'll disappear, and you can then run the commands below.

The board should start blinking red, and appear as a USB-Serial device on your
machine.

[releases]: https://github.com/jamesmunns/soupstone/releases

### Actual Demo

```bash
# Install the Soup CLI
cd host/soup-cli
cargo install -f --path .

# Build the test application, place it into RAM, tell the bootloader
# to run the app, and connect to stdin, stderr, and stdout.
#
# Uses cargo to build, then executes `soup-cli run ELF_FILE`.
cd ../../experiments/soup-app-demo
cargo run --release
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
