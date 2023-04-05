#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use heapless::String;
use soup_stuff::{
    embassy::{
        embassy_executor::Spawner,
        embassy_nrf::{
            self, bind_interrupts,
            gpio::{AnyPin, Level, Output, OutputDrive, Pin},
            peripherals,
            qspi::{
                self, AddressMode, Config, DeepPowerDownConfig, Frequency, Qspi, ReadOpcode,
                SpiMode, WriteOpcode, WritePageSize,
            },
        },
        embassy_time::{Duration, Timer},
    },
    soup_mgr,
    stdio::{stderr, stdin, stdout},
};
use core::fmt::Write;

bind_interrupts!(struct Irqs {
    QSPI => qspi::InterruptHandler<peripherals::QSPI>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    let mut leds = [
        Output::new(p.P0_06.degrade(), Level::Low, OutputDrive::Standard),
        Output::new(p.P0_26.degrade(), Level::High, OutputDrive::Standard),
        Output::new(p.P0_30.degrade(), Level::High, OutputDrive::Standard),
    ];

    let mut cfg = Config::default();
    cfg.read_opcode = ReadOpcode::READ4IO;
    cfg.write_opcode = WriteOpcode::PP4O;   // !
    cfg.xip_offset = 0;
    cfg.write_page_size = WritePageSize::_256BYTES;
    cfg.deep_power_down = Some(DeepPowerDownConfig {
        enter_time: 4096,                      // tDP: 3uS (in units of 16uS)
        exit_time: 4096,                       // tRES2: 8uS (in units of 16uS)
    });                                     // !
    cfg.frequency = Frequency::M32;         // !
    cfg.sck_delay = 10;                     // tSHSL: 30ns, in 62.5ns increments. Overset. // !
    cfg.spi_mode = SpiMode::MODE0;          // 0/3 supported, use 0.
    cfg.address_mode = AddressMode::_24BIT;
    cfg.capacity = 2 * 1024 * 1024;         // 2mbit // !


    let sck = p.P0_21;
    let dat = p.P0_20;
    let csn = p.P0_25;

    // BITBANG WAKEUP FROM DPM JUST IN CASE
    //
    //

    // Keep the CS pin low for 1ms to force it out of sleep mode
    let mut sck = Output::new(sck, Level::Low, OutputDrive::Standard);
    let mut dat = Output::new(dat, Level::Low, OutputDrive::Standard);
    let mut csn = Output::new(csn, Level::Low, OutputDrive::Standard);

    Timer::after(Duration::from_ticks(33)).await;

    let mut msg = 0xABu8;
    for _ in 0..8 {
        if msg & 0x80 == 0 {
            dat.set_low();
        } else {
            dat.set_high();
        }
        msg <<= 1;
        Timer::after(Duration::from_ticks(1)).await;
        sck.set_high();
        Timer::after(Duration::from_ticks(1)).await;
        sck.set_low();
    }

    csn.set_high();
    drop(csn);
    drop(sck);
    drop(dat);
    let csn = unsafe { AnyPin::steal(25) };
    let sck = unsafe { AnyPin::steal(21) };
    let dat = unsafe { AnyPin::steal(20) };


    // END BITBANG

    let qspi = Qspi::new(
        p.QSPI, Irqs, sck, csn, dat, p.P0_24, p.P0_22, p.P0_23, cfg,
    );

    leds[0].set_high();
    leds[2].set_low();

    spawner.spawn(soup_mgr(p.USBD)).ok();
    // spawner.spawn(run1()).ok();
    // spawner.spawn(run2()).ok();
    // spawner.spawn(echo()).ok();
    spawner.spawn(qspi_flash(qspi)).ok();
    spawner.spawn(run3(leds)).ok();
}

#[embassy_executor::task]
async fn qspi_flash(mut q: Qspi<'static, peripherals::QSPI>) {
    let mut sbuffy = String::<256>::new();
    Timer::after(Duration::from_ticks(32768 + 10000)).await;

    //
    // ID
    //

    let mut id = [1; 3];
    if let Err(e) = q.custom_instruction(0x9F, &[], &mut id).await {
        writeln!(&mut sbuffy, "QSPI iderr: {:?}", e).ok();
        stderr().write_bytes_all(sbuffy.as_bytes()).await;
        return
    }
    writeln!(&mut sbuffy, "QSPI id: {:?}", id).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();

    //
    // Read status register
    //
    let mut status = [4; 1];
    if let Err(e) = q.custom_instruction(0x05, &[], &mut status).await {
        writeln!(&mut sbuffy, "QSPI staterr: {:?}", e).ok();
        stderr().write_bytes_all(sbuffy.as_bytes()).await;
        return
    }
    writeln!(&mut sbuffy, "QSPI stat: {:?}", status).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();

    if status[0] & 0x40 == 0 {
        status[0] |= 0x40;

        if let Err(e) = q.custom_instruction(0x01, &status, &mut []).await {
            writeln!(&mut sbuffy, "QSPI quaderr: {:?}", e).ok();
            stderr().write_bytes_all(sbuffy.as_bytes()).await;
            return
        }
        stdout().write_bytes_all(b"Quad enabled.\r\n").await;

        sbuffy.clear();
    }



}

#[embassy_executor::task]
async fn run1() {
    Timer::after(Duration::from_ticks(32768)).await;
    loop {
        stdout().write_bytes_all(b"hello, world!\r\n").await;
        Timer::after(Duration::from_ticks(32768)).await;
    }
}

#[embassy_executor::task]
async fn run2() {
    Timer::after(Duration::from_ticks(40000)).await;
    loop {
        Timer::after(Duration::from_ticks(12 * 32768)).await;
        stderr().write_bytes_all(b"hello, error!\r\n").await;
    }
}

#[embassy_executor::task]
async fn run3(mut leds: [Output<'static, AnyPin>; 3]) {
    let mut idx = 0u32;
    loop {
        Timer::after(Duration::from_ticks(32768 / 2)).await;
        let mut single = idx;
        leds.iter_mut().for_each(|p| {
            if (single & 1) == 1 {
                p.set_low();
            } else {
                p.set_high();
            }
            single >>= 1;
        });
        idx = idx.wrapping_add(1);
    }
}

#[embassy_executor::task]
async fn echo() {
    let mut buf = [0u8; 32];
    loop {
        let n = stdin().read_some(&mut buf).await;
        if n != 0 {
            stdout().write_bytes_all(b"Echo: ").await;
            stdout().write_bytes_all(&buf[..n]).await;
        }
    }
}
