use clap::Parser;
use postcard::{to_stdvec_cobs, accumulator::{CobsAccumulator, FeedResult}};
use std::{time::Duration, io::ErrorKind};


mod cli;
use cli::Stage0;

use stage0_icd::{Request, Response, Error as IcdError};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cmd = Stage0::parse();

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
        eprintln!("Error: No `Stage0 Soupstone` connected!");
        return Ok(());
    };

    let mut port = serialport::new(dport.port_name, 115200)
        .timeout(Duration::from_millis(5))
        .open()
        .map_err(|_| "Error: failed to create port")?;

    type Matchfn = fn(Result<Response, IcdError>) -> bool;

    println!("{cmd:?}");
    let (command, resp_matcher): (Request, Matchfn) = match cmd {
        Stage0::Peek(peek) => match peek.command {
            cli::PeekCommand::PeekU8(pu8) => {
                let req = Request::PeekU8 { addr: pu8.address.0 as usize };

                fn matcher(rsp: Result<Response<'_>, IcdError>) -> bool {
                    matches!(rsp, Ok(Response::PeekBytes { .. }))
                }
                (req, matcher)
            },
        },
        Stage0::Poke(poke) => match poke.command {
            cli::PokeCommand::PokeU8(pu8) => {
                let val = match pu8.val.0.as_slice() {
                    &[a] => a,
                    _ => panic!(),
                };

                let req = Request::PokeU8 { addr: pu8.address.0 as usize, val };

                fn matcher(rsp: Result<Response<'_>, IcdError>) -> bool {
                    matches!(rsp, Ok(Response::Poked { .. }))
                }
                (req, matcher)
            },
        },
    };

    let sercmd = to_stdvec_cobs(&command)?;
    port.write(&[0x00])?;
    port.write(&sercmd)?;

    let mut acc = CobsAccumulator::<512>::new();

    let mut raw_buf = [0u8; 32];

    'outer: loop {
        let mut buf = match port.read(&mut raw_buf) {
            Ok(0) => todo!(),
            Ok(n) => {
                &raw_buf[..n]
            }
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
                FeedResult::Success { data, remaining } => {
                    // Do something with `data: MyData` here.

                    println!("GOT: {data:?}");
                    break 'outer;

                    // remaining
                }
            };
        }
    }

    Ok(())
}
