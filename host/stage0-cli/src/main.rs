use clap::Parser;
use postcard::{
    accumulator::{CobsAccumulator, FeedResult},
    to_stdvec_cobs,
};
use serialport::SerialPort;
use std::{cmp::min, error::Error, io::{ErrorKind, Write, Read}, time::Duration, ops::DerefMut, fs::File};

mod cli;
use cli::{Peek, Stage0, Poke};

use stage0_icd::{Error as IcdError, PeekBytes, Request, Response, managedbuf::Managed, Poked};

fn main() -> Result<(), Box<dyn Error>> {
    let cmd = Stage0::parse();
    let mut port = get_port()?;

    match cmd {
        Stage0::Peek(cmd) => peek(cmd, port.deref_mut()),
        Stage0::Poke(cmd) => poke(cmd, port.deref_mut()),
        Stage0::Bootload(cmd) => {
            send(Request::Bootload { addr: cmd.address.0 }, port.deref_mut())?;
            println!("Sent bootload command.");
            Ok(())
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
            if sn.as_str() == "12345678" {
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

fn peek(cmd: Peek, port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    // Slightly less than the 64 byte packet limit
    const CHUNK_SZ: usize = 256;

    let mut data = Vec::new();
    let mut idx = cmd.address.0 as usize;
    let mut remain = cmd.count;

    while remain != 0 {
        let chunk = min(CHUNK_SZ, remain);
        remain -= chunk;
        send(
            Request::PeekBytes {
                addr: idx,
                len: chunk,
            },
            port,
        )?;
        let resp = recv::<_, PeekBytes<'static>>(
            |r| match r {
                Response::PeekBytes(t) if t.addr == idx => Some(t.to_owned()),
                _ => None,
            },
            port,
        )?;
        data.extend_from_slice(resp.val.as_slice());
        idx += chunk;
    }

    if let Some(f) = cmd.file {
        let mut file = File::create(&f)?;
        file.write_all(&data)?;
    } else {
        data.chunks(16).for_each(|ch| {
            for b in ch {
                print!("{b:02X} ");
            }
            println!();
        });
    }

    Ok(())
}

fn poke(cmd: Poke, port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    // Slightly less than the 64 byte packet limit
    const CHUNK_SZ: usize = 256;

    let mut idx = cmd.address.0 as usize;

    let data = match (cmd.val, cmd.file) {
        (Some(_val), None) => todo!(),
        (None, Some(f)) => {
            let mut file = File::open(&f)?;
            let mut buf = Vec::with_capacity(file.metadata()?.len().try_into()?);
            file.read_to_end(&mut buf)?;
            buf
        },
        (Some(_), Some(_)) => todo!(),
        (None, None) => todo!(),
    };

    let mut remain = &data[..];
    println!("len: {}", remain.len());

    while !remain.is_empty() {
        let chunk_len = min(CHUNK_SZ, remain.len());
        let (chunk, later) = remain.split_at(chunk_len);
        remain = later;
        send(
            Request::PokeBytes {
                addr: idx,
                val: Managed::Borrowed(chunk),
            },
            port,
        )?;
        let _ = recv::<_, Poked>(
            |r| match r {
                Response::Poked(t) if t.addr == idx => Some(Poked { addr: t.addr }),
                _ => None,
            },
            port,
        )?;
        idx += chunk_len;
    }

    Ok(())
}

fn send(cmd: Request<'_>, port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    let sercmd = to_stdvec_cobs(&cmd)?;
    port.write(&[0x00])?;
    port.write(&sercmd)?;
    Ok(())
}

fn recv<F, T>(matcher: F, port: &mut dyn SerialPort) -> Result<T, Box<dyn Error>>
where
    F: Fn(&Response<'_>) -> Option<T>,
{
    let mut acc = CobsAccumulator::<512>::new();

    let mut raw_buf = [0u8; 32];

    'outer: loop {
        let mut buf = match port.read(&mut raw_buf) {
            Ok(0) => todo!(),
            Ok(n) => &raw_buf[..n],
            Err(e) if e.kind() == ErrorKind::TimedOut => {
                continue 'outer;
            }
            Err(e) => {
                panic!("{e:?}");
            }
        };

        'cobs: while !buf.is_empty() {
            buf = match acc.feed_ref::<Result<Response<'_>, IcdError>>(&buf) {
                FeedResult::Consumed => break 'cobs,
                FeedResult::OverFull(new_wind) => new_wind,
                FeedResult::DeserError(new_wind) => new_wind,
                FeedResult::Success { data, .. } => {
                    return match data {
                        Ok(r) => {
                            if let Some(t) = matcher(&r) {
                                Ok(t)
                            } else {
                                Err(format!("Unexpected: {r:?}").into())
                            }
                        }
                        Err(e) => Err(format!("Error: {e:?}").into()),
                    }
                }
            };
        }
    }
}
