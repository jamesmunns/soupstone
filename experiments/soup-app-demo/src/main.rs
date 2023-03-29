#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use embassy_executor::Spawner;
use embassy_nrf::gpio::{AnyPin, Level, Output, OutputDrive, Pin};
use embassy_time::{Duration, Timer};
use soup_app_demo::{
    soup_mgr,
    stdio::{stderr, stdin, stdout},
};

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    let leds = [
        Output::new(p.P0_06.degrade(), Level::High, OutputDrive::Standard),
        Output::new(p.P0_26.degrade(), Level::High, OutputDrive::Standard),
        Output::new(p.P0_30.degrade(), Level::High, OutputDrive::Standard),
    ];

    spawner.spawn(soup_mgr(p.USBD)).ok();
    spawner.spawn(run1()).ok();
    spawner.spawn(run2()).ok();
    spawner.spawn(run3(leds)).ok();
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
    Timer::after(Duration::from_ticks(32768)).await;
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
