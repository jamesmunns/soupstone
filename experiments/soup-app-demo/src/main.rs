#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::mem;

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_nrf::{
    bind_interrupts,
    gpio::{Level, Output, OutputDrive},
    pac,
    peripherals::{self, USBD},
    usb::{
        self,
        vbus_detect::HardwareVbusDetect,
        Driver,
    },
};
use embassy_sync::blocking_mutex::raw::ThreadModeRawMutex;
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, Receiver, Sender, State},
    driver::EndpointError,
    Builder, Config,
};
use embassy_sync::{mutex::Mutex, pipe::Pipe};
use embassy_time::{Duration, Timer};

use panic_reset as _;
use postcard::accumulator::{CobsAccumulator, FeedResult};
use soup_icd::{Control, FromSoup, Managed, ToSoup};

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    POWER_CLOCK => usb::vbus_detect::InterruptHandler;
});

const ACC_SIZE: usize = 512;
static STDOUT: Pipe<ThreadModeRawMutex, 256> = Pipe::new();

fn welp<T>() -> &'static mut T {
    cortex_m::peripheral::SCB::sys_reset();
}

macro_rules! singleton {
    (: $ty:ty = $expr:expr) => {
        cortex_m::singleton!(VAR: $ty = $expr).unwrap_or_else(welp)
    };
}

#[embassy_executor::task]
async fn run1() {
    loop {
        Timer::after(Duration::from_ticks(64000)).await;
        STDOUT.write(b"hello, world!\r\n").await;
    }
}

type UsbSender = Sender<'static, Driver<'static, USBD, HardwareVbusDetect>>;
type UsbReceiver = Receiver<'static, Driver<'static, USBD, HardwareVbusDetect>>;

#[embassy_executor::task]
async fn stdout(tx: &'static Mutex<ThreadModeRawMutex, UsbSender>) {
    let mut scratch_in = [0u8; 32];
    let mut scratch_out = [0u8; 64];

    loop {
        let n = STDOUT.read(&mut scratch_in).await;
        if n == 0 {
            continue;
        }
        let msg = FromSoup::Stdout(Managed::from_borrowed(&scratch_in[..n]));
        if let Ok(sli) = postcard::to_slice_cobs(&msg, &mut scratch_out) {
            let mut tx = tx.lock().await;
            tx.wait_connection().await;
            tx.write_packet(sli).await.ok();
        }
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());

    cortex_m::asm::delay(8_000_000);

    let clock: pac::CLOCK = unsafe { mem::transmute(()) };
    let _led = Output::new(p.P0_13, Level::Low, OutputDrive::Standard);

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
    //
    // Use singletons to reduce stack usage.
    let device_descriptor = singleton!(:[u8; 256] = [0; 256]);
    let config_descriptor = singleton!(:[u8; 256] = [0; 256]);
    let bos_descriptor = singleton!(:[u8; 256] = [0; 256]);
    let msos_descriptor = singleton!(:[u8; 256] = [0; 256]);
    let control_buf = singleton!(:[u8; 64] = [0; 64]);

    let state = singleton!(:State = State::new());

    let mut builder = Builder::new(
        driver,
        config,
        device_descriptor,
        config_descriptor,
        bos_descriptor,
        msos_descriptor,
        control_buf,
    );

    // Create classes on the builder.
    let class = CdcAcmClass::new(&mut builder, state, 64);

    // Build the builder.
    let mut usb = builder.build();

    // Run the USB device.
    let usb_fut = usb.run();

    let (tx, mut rx) = class.split();

    let tx = &*singleton!(: Mutex<ThreadModeRawMutex, UsbSender> = Mutex::new(tx));

    spawner.spawn(stdout(tx)).ok();
    spawner.spawn(run1()).ok();

    // Do stuff with the class!
    let echo_fut = async {
        loop {
            rx.wait_connection().await;
            let _ = minimal(&mut rx, tx).await;
        }
    };

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

async fn minimal(
    rx: &mut UsbReceiver,
    tx: &Mutex<ThreadModeRawMutex, UsbSender>,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    let mut outbuf = [0u8; 512];

    let mut acc = CobsAccumulator::<ACC_SIZE>::new();
    loop {
        let n = rx.read_packet(&mut buf).await?;
        let mut window = &buf[..n];

        'cobs: while !window.is_empty() {
            use FeedResult::*;

            window = match acc.feed_ref::<ToSoup<'_>>(&window) {
                Consumed => break 'cobs,
                OverFull(new_wind) | DeserError(new_wind) => new_wind,
                Success { data, remaining } => {
                    let resp = req_handler(data, &mut outbuf);

                    let mut tx = tx.lock().await;
                    for ch in resp.chunks(64) {
                        tx.wait_connection().await;
                        tx.write_packet(ch).await?;
                    }

                    remaining
                }
            };
        }
    }
}

fn req_handler<'a>(req: ToSoup<'_>, _outbuf: &'a mut [u8]) -> &'a [u8] {
    let _resp: FromSoup<'_> = match req {
        ToSoup::Control(Control::Reboot) => {
            cortex_m::peripheral::SCB::sys_reset();
        }
        _ => todo!(),
    };

    // match postcard::to_slice_cobs(&resp, outbuf) {
    //     Ok(ser) => ser,
    //     Err(_) => {
    //         &[0x00]
    //     },
    // }
}

