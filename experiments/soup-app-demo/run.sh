#!/bin/bash

set -euxo pipefail

shopt -s expand_aliases
alias soup-cli=../../host/soup-cli/target/release/soup-cli

soup-cli stage0 poke -a 0x20000000 -f ./target/demo.bin
soup-cli stage0 bootload -a 0x20000000
soup-cli stdio

set +x

echo "Done."

# cat /dev/serial/by-id/usb-OneVariable_Soup_App_23456789-if00 | xxd
