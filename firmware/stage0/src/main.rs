#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::{
    cell::UnsafeCell,
    mem::{self, MaybeUninit},
};

use cortex_m::singleton;
use defmt::{info, panic};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_nrf::{
    bind_interrupts, pac, peripherals, usb,
    usb::{
        vbus_detect::{HardwareVbusDetect, VbusDetect},
        Driver, Instance,
    },
};
use embassy_usb::{
    class::cdc_acm::{CdcAcmClass, State},
    driver::EndpointError,
    Builder, Config,
};
use postcard::accumulator::{CobsAccumulator, FeedResult};
use stage0_icd::Request;
use {defmt_rtt as _, panic_probe as _};

const SCRATCH_SIZE: usize = 224 * 1024;
const MAGIC_SIZE: usize = 8;
const ACC_SIZE: usize = 512;

#[link_section = ".scratch.SCRATCH"]
#[used]
static SCRATCH: Ram<SCRATCH_SIZE> = Ram::new();

#[link_section = ".magic.MAGIC"]
#[used]
static MAGIC: Ram<MAGIC_SIZE> = Ram::new();

struct Ram<const N: usize> {
    inner: MaybeUninit<UnsafeCell<[u8; N]>>,
}

unsafe impl<const N: usize> Sync for Ram<N> {}
impl<const N: usize> Ram<N> {
    pub const fn new() -> Self {
        Self {
            inner: MaybeUninit::uninit(),
        }
    }

    pub fn as_ptr(&'static self) -> *mut u8 {
        let p: *mut UnsafeCell<[u8; N]> = self.inner.as_ptr().cast_mut();
        let p: *mut u8 = p.cast();
        p
    }
}

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    POWER_CLOCK => usb::vbus_detect::InterruptHandler;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_nrf::init(Default::default());
    let clock: pac::CLOCK = unsafe { mem::transmute(()) };

    info!("Enabling ext hfosc...");
    clock.tasks_hfclkstart.write(|w| unsafe { w.bits(1) });
    while clock.events_hfclkstarted.read().bits() != 1 {}

    // hehehe
    // TODO: removeme
    unsafe {
        SCRATCH.as_ptr().write_bytes(0xFF, SCRATCH_SIZE);
    }

    // Create the driver, from the HAL.
    let driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));

    // Create embassy-usb Config
    let mut config = Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("Soupstone Stage0");
    config.product = Some("USB-serial example");
    config.serial_number = Some("12345678");
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
    let device_descriptor = singleton!(:[u8; 256] = [0; 256]).unwrap_or_else(welp);
    let config_descriptor = singleton!(:[u8; 256] = [0; 256]).unwrap_or_else(welp);
    let bos_descriptor = singleton!(:[u8; 256] = [0; 256]).unwrap_or_else(welp);
    let msos_descriptor = singleton!(:[u8; 256] = [0; 256]).unwrap_or_else(welp);
    let control_buf = singleton!(:[u8; 64] = [0; 64]).unwrap_or_else(welp);

    let mut state = State::new();

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
    let mut class = CdcAcmClass::new(&mut builder, &mut state, 64);

    // Build the builder.
    let mut usb = builder.build();

    // Run the USB device.
    let usb_fut = usb.run();

    // Do stuff with the class!
    let echo_fut = async {
        loop {
            class.wait_connection().await;
            info!("Connected");
            let _ = acc(&mut class).await;
            info!("Disconnected");
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

async fn acc<'d, 'acc, T: Instance + 'd, P: VbusDetect + 'd>(
    class: &mut CdcAcmClass<'d, Driver<'d, T, P>>,
) -> Result<(), Disconnected> {
    let mut buf = [0; 64];
    let mut acc = CobsAccumulator::<ACC_SIZE>::new();
    loop {
        let n = class.read_packet(&mut buf).await?;
        let mut window = &buf[..n];

        'cobs: while !window.is_empty() {
            window = match acc.feed_ref::<Request<'_>>(&window) {
                FeedResult::Consumed => break 'cobs,
                FeedResult::OverFull(new_wind) => new_wind,
                FeedResult::DeserError(new_wind) => new_wind,
                FeedResult::Success { data, remaining } => {
                    // Do something with `data: MyData` here.

                    defmt::debug!("{:?}", data);

                    remaining
                }
            };
        }
    }
}

fn welp<const N: usize>() -> &'static mut [u8; N] {
    loop {
        cortex_m::asm::nop();
    }
}
