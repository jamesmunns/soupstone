#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

use ekv::{flash::PageID, Database};
use heapless::{String, Vec};
use rand_core::RngCore;
use soup_stuff::{
    embassy::{
        embassy_executor::Spawner,
        embassy_nrf::{
            self, bind_interrupts,
            gpio::{AnyPin, Level, Output, OutputDrive, Pin},
            peripherals,
            rng::{self, Rng},
            qspi::{
                self, AddressMode, Config, Frequency, Qspi, ReadOpcode,
                SpiMode, WriteOpcode, WritePageSize,
            },
        },
        embassy_time::{Duration, Timer, Instant}, embassy_sync::blocking_mutex::raw::NoopRawMutex,
    },
    soup_mgr,
    stdio::{stderr, stdin, stdout},
};

#[allow(unused_imports)]
use soup_stuff::embassy::embassy_nrf::qspi::DeepPowerDownConfig;

use core::fmt::Write;

bind_interrupts!(struct Irqs {
    QSPI => qspi::InterruptHandler<peripherals::QSPI>;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

async fn force_qspi_wakeup() {
    // Keep the CS pin low for 1ms to force it out of sleep mode
    let mut sck = Output::new(unsafe { AnyPin::steal(21) }, Level::Low, OutputDrive::Standard);
    let mut dat = Output::new(unsafe { AnyPin::steal(20) }, Level::Low, OutputDrive::Standard);
    let mut csn = Output::new(unsafe { AnyPin::steal(25) }, Level::Low, OutputDrive::Standard);

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
    drop(sck);
    drop(dat);
    core::mem::forget(csn);

    // END BITBANG
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    let mut leds = [
        Output::new(p.P0_06.degrade(), Level::Low, OutputDrive::Standard),
        Output::new(p.P0_26.degrade(), Level::High, OutputDrive::Standard),
        Output::new(p.P0_30.degrade(), Level::High, OutputDrive::Standard),
    ];

    force_qspi_wakeup().await;

    let mut cfg = Config::default();
    cfg.read_opcode = ReadOpcode::READ4IO;
    cfg.write_opcode = WriteOpcode::PP4O;   // !
    cfg.xip_offset = 0;
    cfg.write_page_size = WritePageSize::_256BYTES;
    // cfg.deep_power_down = Some(DeepPowerDownConfig {
    //     enter_time: 4096,                      // tDP: 3uS (in units of 16uS)
    //     exit_time: 4096,                       // tRES2: 8uS (in units of 16uS)
    // });                                     // !
    cfg.frequency = Frequency::M32;          // !
    cfg.sck_delay = 80;                     // tSHSL: 30ns, in 62.5ns increments. Overset. // !
    cfg.spi_mode = SpiMode::MODE0;          // 0/3 supported, use 0.
    cfg.address_mode = AddressMode::_24BIT;
    cfg.capacity = 2 * 1024 * 1024;         // 16Mbit/2MB // !


    let qspi = Qspi::new(
        p.QSPI, Irqs, p.P0_21, p.P0_25, p.P0_20, p.P0_24, p.P0_22, p.P0_23, cfg,
    );
    let rng = Rng::new(p.RNG, Irqs);

    leds[0].set_high();
    leds[2].set_low();

    spawner.spawn(soup_mgr(p.USBD)).ok();
    // spawner.spawn(run1()).ok();
    // spawner.spawn(run2()).ok();
    // spawner.spawn(echo()).ok();
    spawner.spawn(qspi_flash(qspi, rng)).ok();
    spawner.spawn(run3(leds)).ok();
}

#[repr(align(4))]
struct Buf {
    inner: [u8; 4096],
}

struct Qekv<'a> {
    qspi: Qspi<'static, peripherals::QSPI>,
    buf: &'a mut Buf,
}

impl<'a> ekv::flash::Flash for Qekv<'a> {
    type Error = core::convert::Infallible;

    fn page_count(&self) -> usize {
        ekv::config::MAX_PAGE_COUNT
    }

    async fn erase(&mut self, page_id: PageID) -> Result<(), <Qekv<'a> as ekv::flash::Flash>::Error> {
        self.qspi.erase((page_id.index() * ekv::config::PAGE_SIZE) as u32).await.unwrap();
        Ok(())
    }

    async fn read(&mut self, page_id: PageID, offset: usize, data: &mut [u8]) -> Result<(), <Qekv<'a> as ekv::flash::Flash>::Error> {
        let address = page_id.index() * ekv::config::PAGE_SIZE + offset;
        self.qspi.read(address as u32, &mut self.buf.inner[..data.len()]).await.unwrap();
        data.copy_from_slice(&self.buf.inner[..data.len()]);
        Ok(())
    }

    async fn write(&mut self, page_id: PageID, offset: usize, data: &[u8]) -> Result<(), <Qekv<'a> as ekv::flash::Flash>::Error> {
        let address = page_id.index() * ekv::config::PAGE_SIZE + offset;
        self.buf.inner[..data.len()].copy_from_slice(data);
        self.qspi.write(address as u32, &self.buf.inner[..data.len()]).await.unwrap();
        Ok(())
    }
}

#[embassy_executor::task]
async fn qspi_flash(
    mut q: Qspi<'static, peripherals::QSPI>,
    mut rng: Rng<'static, peripherals::RNG>,
) {
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
    writeln!(&mut sbuffy, "QSPI id: {:02X?}", id).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();

    //
    // Read status register
    //
    let mut status = [4; 2];
    if let Err(e) = q.custom_instruction(0x05, &[], &mut status[..1]).await {
        writeln!(&mut sbuffy, "QSPI staterr: {:?}", e).ok();
        stderr().write_bytes_all(sbuffy.as_bytes()).await;
        return
    }
    if let Err(e) = q.custom_instruction(0x35, &[], &mut status[1..]).await {
        writeln!(&mut sbuffy, "QSPI staterr: {:?}", e).ok();
        stderr().write_bytes_all(sbuffy.as_bytes()).await;
        return
    }
    writeln!(&mut sbuffy, "QSPI stat: {:02X?}", status).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();

    // if status[0] & 0x40 == 0 {
    let mut update_stat = false;
    if status[1] & 0x02 == 0 {
        status[1] |= 0x02;
        update_stat = true;
    }
    if status[0] & 0x78 != 0 {
        status[0] &= !0x78;
        update_stat = true;
    }

    if update_stat {
        if let Err(e) = q.custom_instruction(0x01, &status, &mut []).await {
            writeln!(&mut sbuffy, "QSPI quaderr: {:?}", e).ok();
            stderr().write_bytes_all(sbuffy.as_bytes()).await;
            return
        }
        stdout().write_bytes_all(b"Updated status.\r\n").await;
        writeln!(&mut sbuffy, "New QSPI stat: {:02X?}", status).ok();
        stdout().write_bytes_all(sbuffy.as_bytes()).await;

        sbuffy.clear();
    }

    let random_seed = rng.next_u32();

    let mut config = ekv::Config::default();
    config.random_seed = random_seed;

    let mut b = Buf { inner: [0u8; 4096] };

    let mut f = Qekv {
        qspi: q,
        buf: &mut b,
    };

    let db = Database::<_, NoopRawMutex>::new(&mut f, config);


    /////




    writeln!(&mut sbuffy, "Formatting...").ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();

    let start = Instant::now();
    if let Err(_e) = db.format().await {
        stdout().write_bytes_all(b"Format error!\r\n").await;
        return;
    }
    let ms = Instant::now().duration_since(start).as_millis();


    writeln!(&mut sbuffy, "Done in {} ms!", ms).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();


    const KEY_COUNT: usize = 100;
    const TX_SIZE: usize = 10;



    writeln!(&mut sbuffy, "Writing {} keys...", KEY_COUNT).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();

    let start = Instant::now();
    for k in 0..KEY_COUNT / TX_SIZE {
        let mut wtx = db.write_transaction().await;
        for j in 0..TX_SIZE {
            let i = k * TX_SIZE + j;
            let key = make_key(i);
            let val = make_value(i);

            wtx.write(&key, &val).await.unwrap();
        }
        wtx.commit().await.unwrap();
    }
    let ms = Instant::now().duration_since(start).as_millis();


    writeln!(&mut sbuffy, "Done in {} ms! {}ms/key", ms, ms / KEY_COUNT as u64).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();




    writeln!(&mut sbuffy, "Reading {} keys...", KEY_COUNT).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();

    let mut buf = [0u8; 32];
    let start = Instant::now();
    for i in 0..KEY_COUNT {
        let key = make_key(i);
        let val = make_value(i);

        let mut rtx = db.read_transaction().await;
        let n = rtx.read(&key, &mut buf).await.unwrap();
        assert_eq!(&buf[..n], &val[..]);
    }
    let ms = Instant::now().duration_since(start).as_millis();


    writeln!(&mut sbuffy, "Done in {} ms! {}ms/key", ms, ms / KEY_COUNT as u64).ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();




    writeln!(&mut sbuffy, "ALL DONE").ok();
    stdout().write_bytes_all(sbuffy.as_bytes()).await;
    sbuffy.clear();

}

fn make_key(i: usize) -> [u8; 2] {
    (i as u16).to_be_bytes()
}

fn make_value(i: usize) -> Vec<u8, 16> {
    let len = (i * 7) % 16;
    let mut v = Vec::new();
    v.resize(len, 0).unwrap();

    let val = i.to_le_bytes();
    let n = val.len().min(len);
    v[..n].copy_from_slice(&val[..n]);
    v
}

// #[embassy_executor::task]
// async fn qspi_flash(mut q: Qspi<'static, peripherals::QSPI>) {
//     let mut sbuffy = String::<256>::new();
//     Timer::after(Duration::from_ticks(32768 + 10000)).await;

//     //
//     // ID
//     //

//     let mut id = [1; 3];
//     if let Err(e) = q.custom_instruction(0x9F, &[], &mut id).await {
//         writeln!(&mut sbuffy, "QSPI iderr: {:?}", e).ok();
//         stderr().write_bytes_all(sbuffy.as_bytes()).await;
//         return
//     }
//     writeln!(&mut sbuffy, "QSPI id: {:02X?}", id).ok();
//     stdout().write_bytes_all(sbuffy.as_bytes()).await;
//     sbuffy.clear();

//     //
//     // Read status register
//     //
//     let mut status = [4; 2];
//     if let Err(e) = q.custom_instruction(0x05, &[], &mut status[..1]).await {
//         writeln!(&mut sbuffy, "QSPI staterr: {:?}", e).ok();
//         stderr().write_bytes_all(sbuffy.as_bytes()).await;
//         return
//     }
//     if let Err(e) = q.custom_instruction(0x35, &[], &mut status[1..]).await {
//         writeln!(&mut sbuffy, "QSPI staterr: {:?}", e).ok();
//         stderr().write_bytes_all(sbuffy.as_bytes()).await;
//         return
//     }
//     writeln!(&mut sbuffy, "QSPI stat: {:02X?}", status).ok();
//     stdout().write_bytes_all(sbuffy.as_bytes()).await;
//     sbuffy.clear();

//     // if status[0] & 0x40 == 0 {
//     let mut update_stat = false;
//     if status[1] & 0x02 == 0 {
//         status[1] |= 0x02;
//         update_stat = true;
//     }
//     if status[0] & 0x78 != 0 {
//         status[0] &= !0x78;
//         update_stat = true;
//     }

//     if update_stat {
//         if let Err(e) = q.custom_instruction(0x01, &status, &mut []).await {
//             writeln!(&mut sbuffy, "QSPI quaderr: {:?}", e).ok();
//             stderr().write_bytes_all(sbuffy.as_bytes()).await;
//             return
//         }
//         stdout().write_bytes_all(b"Updated status.\r\n").await;
//         writeln!(&mut sbuffy, "New QSPI stat: {:02X?}", status).ok();
//         stdout().write_bytes_all(sbuffy.as_bytes()).await;

//         sbuffy.clear();
//     }

//     #[repr(align(4))]
//     struct Buf {
//         inner: [u8; 4096],
//     }

//     let mut qbuf = Buf { inner: [0u8; 4096] };
//     //
//     // Read some pages
//     //
//     for i in 0..32 {
//         if let Err(_e) = q.read((4096 * i) as u32, &mut qbuf.inner).await {
//             stderr().write_bytes_all(b"Bad read!").await;
//             return;
//         }
//         stdout().write_bytes_all(
//             b"----------------------------------------------------------\r\n"
//         ).await;
//         for (ch_i, ch) in qbuf.inner.chunks(16).enumerate() {
//             sbuffy.clear();
//             write!(&mut sbuffy, "{:08X} : ", (i * 4096) + (16 * ch_i)).ok();
//             for b in ch {
//                 write!(&mut sbuffy, "{:02X} ", b).ok();
//             }
//             writeln!(&mut sbuffy).ok();
//             stdout().write_bytes_all(sbuffy.as_bytes()).await;
//         }
//     }

//     for i in 0..32 {
//         if let Err(_e) = q.erase(4096 * i).await {
//             stderr().write_bytes_all(b"Bad erase\r\n").await;
//             return;
//         } else {
//             stdout().write_bytes_all(b"Erased sectorpagething\r\n").await;
//         }
//     }

//     //
//     // Read some pages
//     //
//     for i in 0..32 {
//         if let Err(_e) = q.read((4096 * i) as u32, &mut qbuf.inner).await {
//             stderr().write_bytes_all(b"Bad read!").await;
//             return;
//         }
//         stdout().write_bytes_all(
//             b"----------------------------------------------------------\r\n"
//         ).await;
//         for (ch_i, ch) in qbuf.inner.chunks(16).enumerate() {
//             sbuffy.clear();
//             write!(&mut sbuffy, "{:08X} : ", (i * 4096) + (16 * ch_i)).ok();
//             for b in ch {
//                 write!(&mut sbuffy, "{:02X} ", b).ok();
//             }
//             writeln!(&mut sbuffy).ok();
//             stdout().write_bytes_all(sbuffy.as_bytes()).await;
//         }
//     }

//     //
//     // Write some pages
//     //
//     for i in 0..32 {
//         qbuf.inner.iter_mut().for_each(|b| *b = 0x42u8.wrapping_add(i as u8));
//         if let Err(_e) = q.write((4096 * i) as u32, &qbuf.inner).await {
//             stderr().write_bytes_all(b"Bad read!").await;
//             return;
//         }
//         stdout().write_bytes_all(b"Wrote.\r\n").await;
//     }

//     //
//     // Read some pages
//     //
//     for i in 0..32 {
//         if let Err(_e) = q.read((4096 * i) as u32, &mut qbuf.inner).await {
//             stderr().write_bytes_all(b"Bad read!").await;
//             return;
//         }
//         stdout().write_bytes_all(
//             b"----------------------------------------------------------\r\n"
//         ).await;
//         for (ch_i, ch) in qbuf.inner.chunks(16).enumerate() {
//             sbuffy.clear();
//             write!(&mut sbuffy, "{:08X} : ", (i * 4096) + (16 * ch_i)).ok();
//             for b in ch {
//                 write!(&mut sbuffy, "{:02X} ", b).ok();
//             }
//             writeln!(&mut sbuffy).ok();
//             stdout().write_bytes_all(sbuffy.as_bytes()).await;
//         }
//     }
// }

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
