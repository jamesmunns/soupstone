use clap::Parser;
use postcard::{
    accumulator::{CobsAccumulator, FeedResult},
    to_stdvec_cobs,
};
use serialport::SerialPort;
use soup_icd::{Managed, ToSoup, FromSoup};
use stage0_icd::{Error as IcdError, PeekBytes, Poked, Request, Response as S0Response};
use std::{
    cmp::min,
    error::Error,
    fmt::Display,
    fs::File,
    io::{ErrorKind, Read, Write},
    ops::DerefMut,
    time::Duration,
};

mod cli;
use cli::{Peek, Poke, Soup, Stage0};

fn main() -> Result<(), Box<dyn Error>> {
    let cmd = Soup::parse();
    let mut last_err: Option<FindError> = None;

    let looking_for = match cmd {
        Soup::Reboot | Soup::Nop | Soup::Stdio => PortKind::SoupApp,
        Soup::Stage0(_) => PortKind::Stage0,
    };

    let mut port = loop {
        println!("Looking for soup device...");
        let (kind, mut port) = loop {
            match (last_err.as_ref(), get_port()) {
                (_, Ok((kind, port))) => {
                    println!(" -> Found {:?}", kind);
                    break (kind, port);
                }
                (Some(FindError::NoneFound), Err(FindError::NoneFound)) => {}
                (Some(FindError::TooManyFound(of)), Err(FindError::TooManyFound(nf)))
                    if of == &nf => {}
                (_, Err(FindError::NoneFound)) => {
                    println!(" -> No soup devices found!");
                    println!(" -> Waiting (hit control-c to stop)...");
                    last_err = Some(FindError::NoneFound);
                }
                (_, Err(FindError::TooManyFound(nf))) => {
                    println!(" -> Too many soup devices found! Remove some.");
                    println!("   -> Found {:?}", nf);
                    println!(" -> Waiting (hit control-c to stop)...");
                    last_err = Some(FindError::TooManyFound(nf));
                }
                (_, Err(FindError::Other(e))) => {
                    println!("unhandled error!");
                    return Err(e);
                }
            }
            // TODO: some kind of notif on new kinds?
            std::thread::sleep(Duration::from_millis(100));
        };

        match (looking_for, kind) {
            (PortKind::Stage0, PortKind::Stage0) => break port,
            (PortKind::SoupApp, PortKind::SoupApp) => break port,

            (PortKind::Stage0, PortKind::SoupApp) => {
                // We found an app, looking for stage 0. Command reset.
                println!(" -> Commanding reset to return to Stage0 Loader.");
                send(ToSoup::Control(soup_icd::Control::Reboot), port.deref_mut())?;
            }
            (PortKind::SoupApp, PortKind::Stage0) => {
                println!(" -> Looking for an application, but found a stage0 loader.");
                println!(" -> Cannot continue.");
                println!(" -> Try Loading an app with `soup-cli stage0 ...` commands.");
                return Err("No application found.".into());
            }
        }
    };

    match cmd {
        Soup::Reboot => {
            println!("Sending reboot command.");
            send(ToSoup::Control(soup_icd::Control::Reboot), port.deref_mut())
        }
        Soup::Nop => {
            println!("Soup App Connected.");
            Ok(())
        },
        Soup::Stage0(shim) => match shim.shim {
            Stage0::Peek(cmd) => peek(cmd, port.deref_mut()),
            Stage0::Poke(cmd) => poke(cmd, port.deref_mut()),
            Stage0::Bootload(cmd) => {
                send(
                    Request::Bootload {
                        addr: cmd.address.0,
                    },
                    port.deref_mut(),
                )?;
                println!("Sent bootload command.");
                Ok(())
            }
        },
        Soup::Stdio => stdio(port.deref_mut()),
    }?;

    Ok(())
}

#[derive(Debug, Copy, Clone)]
pub enum PortKind {
    Stage0,
    SoupApp,
}

#[derive(Debug)]
pub enum FindError {
    NoneFound,
    TooManyFound(String),
    Other(Box<dyn Error>),
}

impl Display for FindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        <Self as core::fmt::Debug>::fmt(self, f)
    }
}

impl From<Box<dyn Error>> for FindError {
    fn from(value: Box<dyn Error>) -> Self {
        Self::Other(value)
    }
}

impl From<&str> for FindError {
    fn from(value: &str) -> Self {
        Self::Other(value.into())
    }
}

impl From<serialport::Error> for FindError {
    fn from(value: serialport::Error) -> Self {
        Self::Other(value.into())
    }
}

impl Error for FindError {}

fn get_port() -> Result<(PortKind, Box<dyn SerialPort>), FindError> {
    let mut ports = vec![];

    for port in serialport::available_ports()? {
        if let serialport::SerialPortType::UsbPort(serialport::UsbPortInfo {
            product: Some(prod),
            ..
        }) = &port.port_type
        {
            match prod.as_str() {
                // TODO(AJM): Something seems to replace spaces with underscores?
                "Stage0_Loader" => {
                    ports.push((PortKind::Stage0, port.clone()));
                }
                // TODO(AJM): change the product name
                x if x.contains("Soup_App") => {
                    ports.push((PortKind::SoupApp, port.clone()));
                }
                _ => {}
            }
        }
    }

    let (kind, dport) = match ports.as_slice() {
        [] => return Err(FindError::NoneFound),
        [one] => one,
        all => {
            let all = all
                .iter()
                .map(|(_kind, p)| p.port_name.clone())
                .collect::<Vec<_>>();
            let all = all.join(", ");
            return Err(FindError::TooManyFound(all));
        }
    };

    let port = serialport::new(&dport.port_name, 115200)
        .timeout(Duration::from_millis(5))
        .open()?;

    Ok((*kind, port))
}

fn stdio(port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    println!("====================");
    println!("Forwarding Stdio... ");
    println!("====================");


    let mut acc = CobsAccumulator::<512>::new();

    let mut raw_buf = [0u8; 32];

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();

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
            buf = match acc.feed_ref::<FromSoup<'_>>(buf) {
                FeedResult::Consumed => break 'cobs,
                FeedResult::OverFull(new_wind) => {
                    println!("OVERFULL ERR");
                    new_wind
                },
                FeedResult::DeserError(new_wind) => {
                    println!("DESER ERR");
                    new_wind
                },
                FeedResult::Success { data, remaining } => {
                    match data {
                        FromSoup::Stdout(r) => {
                            print!("{}", String::from_utf8_lossy(r.as_slice()));
                            stdout.flush()?;
                        },
                        FromSoup::Stderr(r) => {
                            eprint!("{}", String::from_utf8_lossy(r.as_slice()));
                            stderr.flush()?;
                        },
                        FromSoup::ControlResponse(r) => todo!(),
                        FromSoup::FromApp(r) => todo!(),
                        FromSoup::Error(r) => todo!(),
                    }
                    remaining
                }
            };
        }
    }
}

fn send<T: serde::Serialize>(cmd: T, port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    let sercmd = to_stdvec_cobs(&cmd)?;
    port.write_all(&[0x00])?;
    port.write_all(&sercmd)?;
    Ok(())
}

fn recv_s0<F, T>(matcher: F, port: &mut dyn SerialPort) -> Result<T, Box<dyn Error>>
where
    F: Fn(&S0Response<'_>) -> Option<T>,
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
            buf = match acc.feed_ref::<Result<S0Response<'_>, IcdError>>(buf) {
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
        let resp = recv_s0::<_, PeekBytes<'static>>(
            |r| match r {
                S0Response::PeekBytes(t) if t.addr == idx => Some(t.to_owned()),
                _ => None,
            },
            port,
        )?;
        data.extend_from_slice(resp.val.as_slice());
        idx += chunk;
    }

    if let Some(f) = cmd.file {
        let mut file = File::create(f)?;
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
        }
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
        let _ = recv_s0::<_, Poked>(
            |r| match r {
                S0Response::Poked(t) if t.addr == idx => Some(Poked { addr: t.addr }),
                _ => None,
            },
            port,
        )?;
        idx += chunk_len;
    }

    Ok(())
}
