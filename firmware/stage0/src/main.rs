#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::{
    cell::UnsafeCell,
    mem::{self, MaybeUninit}, sync::atomic::{compiler_fence, Ordering},
};

use cortex_m::{singleton, interrupt, peripheral::SCB};

#[cfg(feature = "use-defmt")]
use {defmt_rtt as _, panic_probe as _};

#[cfg(feature = "small")]
use panic_reset as _;
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
use stage0_icd::{Request, Response, Error as IcdError, managedbuf::Managed, PeekBytes, Poked};

const SCRATCH_SIZE: usize = 224 * 1024;
const MAGIC_SIZE: usize = 8;
const ACC_SIZE: usize = 512;

#[cfg(feature = "use-defmt")]
macro_rules! s0log {
    (trace, $($arg:expr),*) => { defmt::trace!($($arg),*) };
    (info, $($arg:expr),*) => { defmt::info!($($arg),*) };
    (debug, $($arg:expr),*) => { defmt::debug!($($arg),*) };
    (error, $($arg:expr),*) => { defmt::error!($($arg),*) };
    (panic, $($arg:expr),*) => { defmt::panic!($($arg),*) };
}

#[cfg(not(feature = "use-defmt"))]
macro_rules! s0log {
    (panic, $($arg:expr),*) => {
        SCB::sys_reset();
    };
    ($level:ident, $($arg:expr),*) => {{ $( let _ = $arg; )* }}
}

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

    pub fn contains(&'static self, addr: usize, len: usize) -> Result<*mut u8, IcdError> {
        let (start, end) = self.start_end();
        let contains = match addr.checked_add(len) {
            Some(range_end) => (addr >= start) && (range_end <= end),
            None => false,
        };
        if contains {
            let offset = addr - start;
            Ok(unsafe { self.as_ptr().add(offset) })
        } else {
            Err(IcdError::AddressOutOfRange {
                request: addr,
                len,
                min: start,
                max: end,
            })
        }
    }

    pub fn start_end(&'static self) -> (usize, usize) {
        let start = self.as_ptr() as usize;
        let end = start.checked_add(N).unwrap();
        (start, end)
    }

    pub fn read_to<'a>(&'static self, start: usize, buf: &'a mut [u8]) -> Result<&'a mut [u8], IcdError> {
        let ptr = self.contains(start, buf.len())?;

        unsafe {
            let len = buf.len();
            compiler_fence(Ordering::SeqCst);

            // TODO: Do we need volatile here?
            core::ptr::copy_nonoverlapping(
                ptr.cast_const(),
                buf.as_mut_ptr(),
                len,
            );

            compiler_fence(Ordering::SeqCst);
        }

        Ok(buf)
    }

    pub fn write_from(&'static self, start: usize, buf: &[u8]) -> Result<(), IcdError> {
        let ptr = self.contains(start, buf.len())?;

        unsafe {
            let len = buf.len();
            compiler_fence(Ordering::SeqCst);

            // TODO: Do we need volatile here?
            core::ptr::copy_nonoverlapping(
                buf.as_ptr(),
                ptr,
                len,
            );

            compiler_fence(Ordering::SeqCst);
        }

        Ok(())
    }
}

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<peripherals::USBD>;
    POWER_CLOCK => usb::vbus_detect::InterruptHandler;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let (magic_1, magic_2) = unsafe {
        let m = MAGIC.as_ptr().cast::<u32>();
        let ret = (m.read_unaligned(), m.add(1).read_unaligned());
        m.write_unaligned(0);
        m.add(1).write_unaligned(0);
        ret
    };

    match (magic_1, magic_2) {
        (0x0FACADE0, addr) => unsafe {
            // bootload!
            let scb: SCB = mem::transmute(());
            scb.vtor.write(addr);
            cortex_m::asm::bootload(addr as *const u32);
        }
        _ => {
            // Do nothing!
        }
    }

    let p = embassy_nrf::init(Default::default());
    let clock: pac::CLOCK = unsafe { mem::transmute(()) };
    let nvmc: pac::NVMC = unsafe { mem::transmute(()) };
    nvmc.icachecnf.write(|w| w.cacheen().set_bit());
    cortex_m::asm::isb();

    s0log!(info, "Enabling ext hfosc...");
    clock.tasks_hfclkstart.write(|w| unsafe { w.bits(1) });
    while clock.events_hfclkstarted.read().bits() != 1 {}

    // Create the driver, from the HAL.
    let driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));

    // Create embassy-usb Config
    let mut config = Config::new(0xc0de, 0xcafe);
    config.manufacturer = Some("OneVariable");
    config.product = Some("Stage0 Loader");
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
            s0log!(info, "Connected");
            let _ = acc(&mut class).await;
            s0log!(info, "Disconnected");
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
    let mut outbuf = [0u8; 512];

    let mut acc = CobsAccumulator::<ACC_SIZE>::new();
    loop {
        let n = class.read_packet(&mut buf).await?;
        let mut window = &buf[..n];

        'cobs: while !window.is_empty() {
            use FeedResult::*;

            window = match acc.feed_ref::<Request<'_>>(&window) {
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
    }
}

fn req_handler<'a>(req: Request<'_>, outbuf: &'a mut [u8]) -> &'a [u8] {
    let mut membuf = [0u8; 256];

    let resp: Result<Response<'_>, IcdError> = match req {
        Request::PeekBytes { addr, len } => {
            if len > membuf.len() {
                Err(IcdError::RangeTooLarge {
                    request: len,
                    max: membuf.len(),
                })
            } else {
                SCRATCH.read_to(addr, &mut membuf[..len])
                    .map(|buf| {
                        Response::PeekBytes(PeekBytes { addr, val: Managed::from_borrowed(buf) })
                    })
            }
        },
        Request::PokeBytes { addr, val } => {
            SCRATCH.write_from(addr, val.as_slice())
                .map(|()| {
                    Response::Poked(Poked { addr })
                })
        },
        Request::Bootload { addr } => {
            // Validation is for suckers.
            let mut scratch = [0u8; 8];
            scratch[..4].copy_from_slice(&0x0FACADE0u32.to_ne_bytes());
            scratch[4..].copy_from_slice(&addr.to_ne_bytes());
            unsafe {
                MAGIC.as_ptr().copy_from_nonoverlapping(scratch.as_ptr(), 8);
            }

            // o7
            interrupt::disable();
            SCB::sys_reset();
        }

        // Unimplemented
        // Request::PeekU16 { addr } => todo!(),
        // Request::PeekU32 { addr } => todo!(),
        // Request::PokeU16 { addr, val } => todo!(),
        // Request::PokeU32 { addr, val } => todo!(),
        // Request::ClearMagic => todo!(),
        // Request::Reboot => todo!(),
        _ => todo!()
    };

    match postcard::to_slice_cobs(&resp, outbuf) {
        Ok(ser) => ser,
        Err(_) => {
            s0log!(error, "Serialization went bad.");
            &[0x00]
        },
    }
}

fn welp<const N: usize>() -> &'static mut [u8; N] {
    loop {
        cortex_m::asm::nop();
    }
}
