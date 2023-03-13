#!/bin/bash

set -euxo pipefail

# Just in case it's still running
soup-cli reboot && sleep 1 || :

# Load the new firmware
stage0-cli poke -a 0x20000000 -f ./target/demo.bin
stage0-cli bootload -a 0x20000000

sleep 1

cat /dev/serial/by-id/usb-OneVariable_Soup_App_Demo_23456789-if00 | xxd
