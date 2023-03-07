#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use defmt::{info, unwrap};
use embassy_executor::Spawner;
use embassy_nrf::gpio::{AnyPin, Input, Pin as _, Pull};
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::task(pool_size = 4)]
async fn button_task(n: usize, mut pin: Input<'static, AnyPin>) {
    loop {
        pin.wait_for_low().await;
        info!("Button {:?} pressed!", n);
        pin.wait_for_high().await;
        info!("Button {:?} released!", n);
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());
    info!("Starting!");

    let btn1 = Input::new(p.P0_11.degrade(), Pull::Up);
    let btn2 = Input::new(p.P0_12.degrade(), Pull::Up);
    let btn3 = Input::new(p.P0_24.degrade(), Pull::Up);
    let btn4 = Input::new(p.P0_25.degrade(), Pull::Up);

    unwrap!(spawner.spawn(button_task(1, btn1)));
    unwrap!(spawner.spawn(button_task(2, btn2)));
    unwrap!(spawner.spawn(button_task(3, btn3)));
    unwrap!(spawner.spawn(button_task(4, btn4)));
}
