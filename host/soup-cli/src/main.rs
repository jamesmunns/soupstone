use clap::Parser;
use postcard::{
    accumulator::{CobsAccumulator, FeedResult},
    to_stdvec_cobs,
};
use serialport::SerialPort;
use soup_icd::{FromSoup, Managed, ToSoup};
use stage0_icd::{Error as IcdError, PeekBytes, Poked, Request, Response as S0Response};
use std::{
    cmp::min,
    error::Error,
    fs::File,
    io::{ErrorKind, Read, Write},
    ops::DerefMut,
    sync::mpsc::channel,
};

mod cli;
mod elf;
mod port;

use crate::{
    cli::{Peek, Poke, Run, Soup, Stage0, WriteBytes},
    elf::parse_loadable,
    port::{connect, PortKind},
};

fn main() -> Result<(), Box<dyn Error>> {
    let cmd = Soup::parse();

    match cmd {
        Soup::Reboot => {
            println!("Sending reboot command.");
            let mut port = connect(PortKind::SoupApp)?;
            send(ToSoup::Control(soup_icd::Control::Reboot), port.deref_mut())
        }
        Soup::Nop => {
            println!("Soup App Connected.");
            Ok(())
        }
        Soup::Stage0(shim) => {
            let mut port = connect(PortKind::Stage0)?;
            match shim.shim {
                Stage0::Peek(cmd) => peek(cmd, port.deref_mut()),
                Stage0::Poke(cmd) => poke(cmd, port.deref_mut()).map(drop),
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
                Stage0::FlashPeek(cmd) => flash_peek(cmd, port.deref_mut()),
                Stage0::FlashPoke(cmd) => flash_poke(cmd, port.deref_mut()),
            }
        }
        Soup::Stdio => {
            let mut port = connect(PortKind::SoupApp)?;
            stdio(port.deref_mut())
        }
        Soup::Run(Run { elf_path }) => run(elf_path),
    }?;

    Ok(())
}

fn run(path: String) -> Result<(), Box<dyn Error>> {
    let load = parse_loadable(path)?;
    let mut port = connect(PortKind::Stage0)?;

    // Poke elf file into memory
    poke(
        Poke {
            address: cli::Address(load.addr),
            val: Some(WriteBytes(load.data)),
            file: None,
        },
        port.deref_mut(),
    )?;

    // Bootload
    send(Request::Bootload { addr: load.addr }, port.deref_mut())?;
    println!("Sent bootload command.");

    // Drop the port, reconnect as an app, attach to stdio
    drop(port);

    let mut port = connect(PortKind::SoupApp)?;
    stdio(port.deref_mut())?;

    Ok(())
}

fn flash_poke(cmd: Poke, port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    // First, poke the data into RAM, but start at the origin.
    let mut ram_poke = cmd.clone();
    ram_poke.address = cli::Address(0x2000_0000);
    println!(" -> Sending to RAM...");
    let len = poke(ram_poke, port)?;

    // Then, send a ram copy command
    let copy_cmd = Request::FlashCopy {
        ram_start: 0x2000_0000,
        flash_start: cmd.address.0 as usize,
        len,
    };
    send(copy_cmd, port)?;

    println!(" -> Sent RAM->Flash copy command");

    let _ = recv_s0::<_, ()>(
        |r| match r {
            S0Response::FlashCopied => Some(()),
            _ => None,
        },
        port,
    )?;

    println!(" -> Completed!");

    Ok(())
}

fn stdio(port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    println!("====================");
    println!("Forwarding Stdio... ");
    println!("====================");

    let mut acc = CobsAccumulator::<512>::new();

    let mut raw_buf = [0u8; 32];

    let mut stdout = std::io::stdout();
    let mut stderr = std::io::stderr();
    let mut stdin = std::io::stdin();

    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut buf = [0u8; 32];
        loop {
            match stdin.read(&mut buf) {
                Ok(n) => {
                    tx.send((&buf[..n]).to_vec()).unwrap();
                }
                Err(_) => todo!(),
            }
        }
    });

    loop {
        let mut buf = match port.read(&mut raw_buf) {
            Ok(0) => todo!(),
            Ok(n) => &raw_buf[..n],
            Err(e) if e.kind() == ErrorKind::TimedOut => &[],
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
                }
                FeedResult::DeserError(new_wind) => {
                    println!("DESER ERR");
                    new_wind
                }
                FeedResult::Success { data, remaining } => {
                    match data {
                        FromSoup::Stdout(r) => {
                            print!("{}", String::from_utf8_lossy(r.as_slice()));
                            stdout.flush()?;
                        }
                        FromSoup::Stderr(r) => {
                            eprint!("{}", String::from_utf8_lossy(r.as_slice()));
                            stderr.flush()?;
                        }
                        FromSoup::ControlResponse(_r) => todo!(),
                        FromSoup::FromApp(_r) => todo!(),
                        FromSoup::Error(_r) => todo!(),
                    }
                    remaining
                }
            };
        }

        if let Ok(v) = rx.try_recv() {
            let msg = ToSoup::Stdin(Managed::Borrowed(&v));
            send(msg, port).unwrap();
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

fn flash_peek(cmd: Peek, port: &mut dyn SerialPort) -> Result<(), Box<dyn Error>> {
    // Slightly less than the 64 byte packet limit
    const CHUNK_SZ: usize = 256;

    let mut data = Vec::new();
    let mut idx = cmd.address.0 as usize;
    let mut remain = cmd.count;

    while remain != 0 {
        let chunk = min(CHUNK_SZ, remain);
        remain -= chunk;
        send(
            Request::PeekBytesFlash {
                addr: idx,
                len: chunk,
            },
            port,
        )?;
        let resp = recv_s0::<_, PeekBytes<'static>>(
            |r| match r {
                S0Response::PeekBytesFlash(t) if t.addr == idx => Some(t.to_owned()),
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

fn poke(cmd: Poke, port: &mut dyn SerialPort) -> Result<usize, Box<dyn Error>> {
    // Slightly less than the 64 byte packet limit
    const CHUNK_SZ: usize = 256;

    let mut idx = cmd.address.0 as usize;

    let data = match (cmd.val, cmd.file) {
        (Some(val), None) => val.0,
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
    let ttl = remain.len();
    println!("   -> len: {}", ttl);

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

    Ok(ttl)
}
