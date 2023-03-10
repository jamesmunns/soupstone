# It's something

## Current repro steps

```bash
# build the demo app
cd experiments/stage0-ram-payload/
cargo objcopy --release -- -O binary demo.bin
cp ./demo.bin ../../host/stage0-cli/demo.bin

# Flash the stage0 loader
cd ../../firmware/stage0/
cargo run --release

# hit control-c, hit the reset button on the board
# ...

# Run the loader app
cd ../../host/stage0-cli/
cargo run --release -- poke -a 0x20000000 -f demo.bin
cargo run --release -- bootload -a 0x20000000

# Open up the tty, typing echos and makes the led blink
screen /dev/serial/by-id/usb-Embassy_USB-serial_example_12345678-if00
```
