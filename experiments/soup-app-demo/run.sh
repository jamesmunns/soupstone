#!/bin/bash

set -euxo pipefail

soup-cli stage0 poke -a 0x20000000 -f ./target/demo.bin
soup-cli stage0 bootload -a 0x20000000
soup-cli nop

set +x

echo "Done."

# cat /dev/serial/by-id/usb-OneVariable_Soup_App_23456789-if00 | xxd
