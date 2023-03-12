#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![allow(unused_variables, unused_imports, unused_mut)]

use core::mem;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_futures::select::{select, Either};
use embassy_nrf::gpio::{Output, Level, OutputDrive};
use embassy_nrf::peripherals::P0_13;
use embassy_nrf::usb::vbus_detect::{HardwareVbusDetect, VbusDetect};
use embassy_nrf::usb::{Driver, Instance};
use embassy_nrf::{bind_interrupts, pac, peripherals, usb};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config};
use panic_reset as _;
use postcard::accumulator::{CobsAccumulator, FeedResult};
use soup_icd::{ToSoup, FromSoup, Control};
use embassy_time::{Duration, Timer};
use embassy_sync::pipe::Pipe;
use embassy_sync::mutex::Mutex;

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    POWER_CLOCK => usb::vbus_detect::InterruptHandler;
});

const ACC_SIZE: usize = 512;
static STDOUT: Pipe<CriticalSectionRawMutex, 256> = Pipe::new();

#[embassy_executor::task]
async fn run1() {
    loop {
        Timer::after(Duration::from_ticks(64000)).await;
        STDOUT.write(b"hello, world!\r\n").await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    cortex_m::asm::delay(8_000_000);

    let clock: pac::CLOCK = unsafe { mem::transmute(()) };
    let mut led = Output::new(p.P0_13, Level::Low, OutputDrive::Standard);

    // info!("Enabling ext hfosc...");
    clock.tasks_hfclkstart.write(|w| unsafe { w.bits(1) });
    while clock.events_hfclkstarted.read().bits() != 1 {}

    // Create the driver, from the HAL.
    let driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));

    // Create embassy-usb Config
    let mut config = Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("OneVariable");
    config.product = Some("Soup App Demo");
    config.serial_number = Some("23456789");
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    // Required for windows compatiblity.
    // https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
    config.device_class = 0xEF;
    config.device_sub_class = 0x02;
    config.device_protocol = 0x01;
    config.composite_with_iads = true;

    // Create embassy-usb DeviceBuilder using the driver and config.
    // It needs some buffers for building the descriptors.
    let mut device_descriptor = [0; 256];
    let mut config_descriptor = [0; 256];
    let mut bos_descriptor = [0; 256];
    let mut msos_descriptor = [0; 256];
    let mut control_buf = [0; 64];

    let mut state = State::new();

    let mut builder = Builder::new(
        driver,
        config,
        &mut device_descriptor,
        &mut config_descriptor,
        &mut bos_descriptor,
        &mut msos_descriptor,
        &mut control_buf,
    );

    // Create classes on the builder.
    let mut class = CdcAcmClass::new(&mut builder, &mut state, 64);

    // Build the builder.
    let mut usb = builder.build();

    // Run the USB device.
    let usb_fut = usb.run();

    // Do stuff with the class!
    let echo_fut = async {
        loop {
            class.wait_connection().await;
            // info!("Connected");
            let _ = minimal(&mut class).await;
            // info!("Disconnected");
        }
    };

    spawner.spawn(run1()).ok();

    // Run everything concurrently.
    // If we had made everything `'static` above instead, we could do this using separate tasks instead.
    join(usb_fut, echo_fut).await;
}

struct Disconnected {}

impl From<EndpointError> for Disconnected {
    fn from(val: EndpointError) -> Self {
        match val {
            EndpointError::BufferOverflow => panic!("Buffer overflow"),
            EndpointError::Disabled => Disconnected {},
        }
    }
}

async fn minimal<'d, 'acc, T: Instance + 'd, P: VbusDetect + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T, P>>,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    let mut stdout = [0; 32];
    let mut outbuf = [0u8; 512];

    let mut acc = CobsAccumulator::<ACC_SIZE>::new();
    loop {
        match select(STDOUT.read(&mut stdout), class.read_packet(&mut buf)).await {
            Either::First(out) => {
                if out != 0 {
                    if let Ok(sern) = postcard::to_slice_cobs(
                        &FromSoup::Stdout(soup_icd::Managed::Borrowed(&stdout[..out])),
                        &mut outbuf,
                    ) {
                        class.write_packet(sern).await?;
                    }
                }
            },
            Either::Second(read) => {
                let n = read?;
                let mut window = &buf[..n];

                'cobs: while !window.is_empty() {
                    use FeedResult::*;

                    window = match acc.feed_ref::<ToSoup<'_>>(&window) {
                        Consumed => break 'cobs,
                        OverFull(new_wind) | DeserError(new_wind) => new_wind,
                        Success { data, remaining } => {
                            let resp = req_handler(data, &mut outbuf);

                            for ch in resp.chunks(64) {
                                class.write_packet(ch).await?;
                            }

                            remaining
                        }
                    };
                }
            },
        }

    }
}

fn req_handler<'a>(req: ToSoup<'_>, _outbuf: &'a mut [u8]) -> &'a [u8] {
    let _resp: FromSoup<'_> = match req {
        ToSoup::Control(Control::Reboot) => {
            cortex_m::peripheral::SCB::sys_reset();
        },
        _ => todo!(),
    };


    // match postcard::to_slice_cobs(&resp, outbuf) {
    //     Ok(ser) => ser,
    //     Err(_) => {
    //         &[0x00]
    //     },
    // }
}
