#![no_std]
#![feature(type_alias_impl_trait)]

use core::mem;

use embassy_executor::Spawner;
use embassy_nrf::{
    bind_interrupts, pac,
    peripherals::{self, USBD},
    usb::{self, vbus_detect::HardwareVbusDetect, Driver},
};
use embassy_sync::{blocking_mutex::raw::ThreadModeRawMutex, mutex::Mutex, pipe::Pipe};
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, Receiver, Sender, State},
    driver::EndpointError,
    Builder, Config, UsbDevice,
};

use panic_reset as _;
use postcard::accumulator::{CobsAccumulator, FeedResult};
use soup_icd::{Control, FromSoup, Managed, ToSoup};

pub mod embassy {
    pub use embassy_executor;
    pub use embassy_nrf;
    pub use embassy_usb;
    pub use embassy_time;
    pub use embassy_sync;
}

const ACC_SIZE: usize = 512;
static STDOUT: Pipe<ThreadModeRawMutex, 256> = Pipe::new();
static STDIN: Pipe<ThreadModeRawMutex, 256> = Pipe::new();
static STDERR: Pipe<ThreadModeRawMutex, 256> = Pipe::new();

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    POWER_CLOCK => usb::vbus_detect::InterruptHandler;
});

fn welp<T>() -> &'static mut T {
    cortex_m::peripheral::SCB::sys_reset();
}

macro_rules! singleton {
    (: $ty:ty = $expr:expr) => {
        cortex_m::singleton!(VAR: $ty = $expr).unwrap_or_else(welp)
    };
}

type UsbSender = Sender<'static, Driver<'static, USBD, HardwareVbusDetect>>;
type UsbReceiver = Receiver<'static, Driver<'static, USBD, HardwareVbusDetect>>;

pub mod stdio {
    use super::*;

    pub struct Stdout;
    pub struct Stderr;
    pub struct Stdin;

    static STDOUT_LOCK: Mutex<ThreadModeRawMutex, &'static Pipe<ThreadModeRawMutex, 256>> =
        Mutex::new(&STDOUT);
    static STDERR_LOCK: Mutex<ThreadModeRawMutex, &'static Pipe<ThreadModeRawMutex, 256>> =
        Mutex::new(&STDERR);
    static STDIN_LOCK: Mutex<ThreadModeRawMutex, &'static Pipe<ThreadModeRawMutex, 256>> =
        Mutex::new(&STDIN);

    impl Stdout {
        pub async fn write_bytes_all(&self, mut s: &[u8]) {
            let stdout = STDOUT_LOCK.lock().await;
            while !s.is_empty() {
                let n = stdout.write(s).await;
                let (_now, later) = s.split_at(n);
                s = later;
            }
        }
    }

    impl Stderr {
        pub async fn write_bytes_all(&self, mut s: &[u8]) {
            let stdout = STDERR_LOCK.lock().await;
            while !s.is_empty() {
                let n = stdout.write(s).await;
                let (_now, later) = s.split_at(n);
                s = later;
            }
        }
    }

    impl Stdin {
        pub async fn read_some(&self, buf: &mut [u8]) -> usize {
            let stdin = STDIN_LOCK.lock().await;

            // Read once async
            let taken = stdin.read(buf).await;
            let (_now, later) = buf.split_at_mut(taken);

            // Then once nonblocking (to catch wraparounds)
            match stdin.try_read(later) {
                Ok(n) => taken + n,
                Err(_) => taken,
            }
        }

        pub fn try_read_pending(&self, mut buf: &mut [u8]) -> usize {
            let mut taken = 0;
            let stdin = match STDIN_LOCK.try_lock() {
                Ok(s) => s,
                Err(_) => return 0,
            };

            while !buf.is_empty() {
                match stdin.try_read(buf) {
                    Ok(n) => {
                        taken += n;
                        let (_now, later) = buf.split_at_mut(n);
                        buf = later;
                    }
                    Err(_) => return taken,
                }
            }

            taken
        }
    }

    pub fn stdout() -> Stdout {
        Stdout
    }

    pub fn stderr() -> Stderr {
        Stderr
    }

    pub fn stdin() -> Stdin {
        Stdin
    }
}

#[embassy_executor::task]
async fn stdout(tx: &'static Mutex<ThreadModeRawMutex, UsbSender>) {
    let mut scratch_in = [0u8; 32];
    let mut scratch_out = [0u8; 64];

    loop {
        let n = STDOUT.read(&mut scratch_in).await;
        if n != 0 {
            let msg = FromSoup::Stdout(Managed::from_borrowed(&scratch_in[..n]));
            if let Ok(sli) = postcard::to_slice_cobs(&msg, &mut scratch_out) {
                let mut tx = tx.lock().await;
                tx.wait_connection().await;
                tx.write_packet(sli).await.ok();
            }
        }
    }
}

#[embassy_executor::task]
async fn stderr(tx: &'static Mutex<ThreadModeRawMutex, UsbSender>) {
    let mut scratch_in = [0u8; 32];
    let mut scratch_out = [0u8; 64];

    loop {
        let n = STDERR.read(&mut scratch_in).await;
        if n != 0 {
            let msg = FromSoup::Stderr(Managed::from_borrowed(&scratch_in[..n]));
            if let Ok(sli) = postcard::to_slice_cobs(&msg, &mut scratch_out) {
                let mut tx = tx.lock().await;
                tx.wait_connection().await;
                tx.write_packet(sli).await.ok();
            }
        }
    }
}

#[embassy_executor::task]
async fn usb_task(mut usb: UsbDevice<'static, Driver<'static, USBD, HardwareVbusDetect>>) {
    usb.run().await;
}

#[embassy_executor::task]
pub async fn soup_mgr(usb: USBD) {
    let clock: pac::CLOCK = unsafe { mem::transmute(()) };
    let spawner = Spawner::for_current_executor().await;

    clock.tasks_hfclkstart.write(|w| unsafe { w.bits(1) });
    while clock.events_hfclkstarted.read().bits() != 1 {}

    // Create the driver, from the HAL.
    let driver = Driver::new(usb, Irqs, HardwareVbusDetect::new(Irqs));

    // Create embassy-usb Config
    let mut config = Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("OneVariable");
    config.product = Some("Soup App");
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
    let device_descriptor = singleton!(:[u8; 24] = [0; 24]);
    let config_descriptor = singleton!(:[u8; 96] = [0; 96]);
    let bos_descriptor = singleton!(:[u8; 16] = [0; 16]);
    let msos_descriptor = singleton!(:[u8; 0] = [0; 0]);
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
    let usb = builder.build();

    let (tx, mut rx) = class.split();

    let tx = &*singleton!(: Mutex<ThreadModeRawMutex, UsbSender> = Mutex::new(tx));

    spawner.spawn(stdout(tx)).ok();
    spawner.spawn(stderr(tx)).ok();
    spawner.spawn(usb_task(usb)).ok();

    // Do stuff with the class!
    let soup_comms = async {
        loop {
            rx.wait_connection().await;
            let _ = minimal(&mut rx, tx).await;
        }
    };

    // Run everything concurrently.
    // If we had made everything `'static` above instead, we could do this using separate tasks instead.
    soup_comms.await;
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
                    let resp = req_handler(data, &mut outbuf).await;

                    if !resp.is_empty() {
                        let mut tx = tx.lock().await;
                        for ch in resp.chunks(64) {
                            tx.wait_connection().await;
                            tx.write_packet(ch).await?;
                        }
                    }

                    remaining
                }
            };
        }
    }
}

async fn req_handler<'a>(req: ToSoup<'_>, outbuf: &'a mut [u8]) -> &'a [u8] {
    let resp: Option<FromSoup<'_>> = match req {
        ToSoup::Control(Control::Reboot) => {
            cortex_m::peripheral::SCB::sys_reset();
        }
        ToSoup::Control(Control::SendAppInfo) => None,
        ToSoup::Stdin(si) => {
            STDIN.write(si.as_slice()).await;
            None
        }
        ToSoup::ToApp(_) => todo!(),
    };

    if let Some(res) = resp {
        match postcard::to_slice_cobs(&res, outbuf) {
            Ok(ser) => ser,
            Err(_) => &[0x00],
        }
    } else {
        &[]
    }
}
