use clap::Parser;
use postcard::{
    accumulator::{CobsAccumulator, FeedResult},
    to_stdvec_cobs,
};
use serialport::SerialPort;
use soup_icd::ToSoup;
use std::{cmp::min, error::Error, io::{ErrorKind, Write, Read}, time::Duration, ops::DerefMut, fs::File};

mod cli;
use cli::Soup;

// use stage0_icd::{Error as IcdError, PeekBytes, Request, Response, managedbuf::Managed, Poked};

fn main() -> Result<(), Box<dyn Error>> {
    let cmd = Soup::parse();
    let mut port = get_port()?;

    match cmd {
        Soup::Reboot => {
            send(ToSoup::Control(soup_icd::Control::Reboot), port.deref_mut())
        }
    }?;

    Ok(())
}

fn get_port() -> Result<Box<dyn SerialPort>, Box<dyn Error>> {
    let mut dport = None;

    for port in serialport::available_ports().unwrap() {
        if let serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
            serial_number: Some(sn),
            ..
        }) = &port.port_type
        {
            if sn.as_str() == "23456789" {
                dport = Some(port.clone());
                break;
            }
        }
    }

    let dport = if let Some(port) = dport {
        port
    } else {
        return Err("Error: No `Stage0 Soupstone` connected!".into());
    };

    let port = serialport::new(dport.port_name, 115200)
        .timeout(Duration::from_millis(5))
        .open()
        .map_err(|_| "Error: failed to create port")?;

    Ok(port)
}

fn send(cmd: ToSoup<'_>, port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    let sercmd = to_stdvec_cobs(&cmd)?;
    port.write(&[0x00])?;
    port.write(&sercmd)?;
    Ok(())
}

// fn recv<F, T>(matcher: F, port: &mut dyn SerialPort) -> Result<T, Box<dyn Error>>
// where
//     F: Fn(&Response<'_>) -> Option<T>,
// {
//     let mut acc = CobsAccumulator::<512>::new();

//     let mut raw_buf = [0u8; 32];

//     'outer: loop {
//         let mut buf = match port.read(&mut raw_buf) {
//             Ok(0) => todo!(),
//             Ok(n) => &raw_buf[..n],
//             Err(e) if e.kind() == ErrorKind::TimedOut => {
//                 continue 'outer;
//             }
//             Err(e) => {
//                 panic!("{e:?}");
//             }
//         };

//         'cobs: while !buf.is_empty() {
//             buf = match acc.feed_ref::<Result<Response<'_>, IcdError>>(&buf) {
//                 FeedResult::Consumed => break 'cobs,
//                 FeedResult::OverFull(new_wind) => new_wind,
//                 FeedResult::DeserError(new_wind) => new_wind,
//                 FeedResult::Success { data, .. } => {
//                     return match data {
//                         Ok(r) => {
//                             if let Some(t) = matcher(&r) {
//                                 Ok(t)
//                             } else {
//                                 Err(format!("Unexpected: {r:?}").into())
//                             }
//                         }
//                         Err(e) => Err(format!("Error: {e:?}").into()),
//                     }
//                 }
//             };
//         }
//     }
// }
