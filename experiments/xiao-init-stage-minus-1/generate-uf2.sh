#!/bin/bash

set -euxo pipefail

cargo objcopy --release -- -O binary ./target/minus-1.bin
uf2conv target/minus-1.bin --base 0x27000 -f 0xADA52840 --output ./target/minus-1.uf2

set +x

echo "Okay now copy './target/minus-1.uf2' to the flash drive"
