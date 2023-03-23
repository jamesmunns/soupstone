#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::mem;

use embassy_executor::Spawner;
use embassy_nrf::gpio::{Level, Output, OutputDrive};
use embassy_nrf::{nvmc::Nvmc, pac};
use embedded_storage::nor_flash::{ErrorType, NorFlash};
use panic_reset as _;

const STAGE0_PAYLOAD: &'static [u8] = include_bytes!("../stage0.bin");

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());
    let clock: pac::CLOCK = unsafe { mem::transmute(()) };

    let mut led_2 = Output::new(p.P0_14, Level::High, OutputDrive::Standard);
    let mut led_4 = Output::new(p.P0_16, Level::High, OutputDrive::Standard);
    let mut nvmc = Nvmc::new(p.NVMC);

    clock.tasks_hfclkstart.write(|w| unsafe { w.bits(1) });
    while clock.events_hfclkstarted.read().bits() != 1 {}

    // Start at top of flash
    let mut idx: u32 = 0;

    let res = STAGE0_PAYLOAD
        .chunks(Nvmc::ERASE_SIZE)
        .try_for_each(|ch| {
            nvmc.erase(idx, idx + (Nvmc::ERASE_SIZE as u32))?;
            let aligned_end = ch.len() & !(Nvmc::WRITE_SIZE - 1);
            let (aligned, unaligned) = ch.split_at(aligned_end);
            nvmc.write(idx, aligned)?;

            if !unaligned.is_empty() {
                let mut extra = [0xFF; Nvmc::WRITE_SIZE];
                extra[..unaligned.len()].copy_from_slice(unaligned);
                nvmc.write(idx + (aligned.len() as u32), &extra)?;
            }

            idx += ch.len() as u32;

            Result::<_, <Nvmc as ErrorType>::Error>::Ok(())
        });

    if res.is_ok() {
        led_4.set_low();
    } else {
        led_2.set_low();
    }

    loop {
        cortex_m::asm::wfi();
    }
}
