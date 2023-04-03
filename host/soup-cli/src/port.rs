use std::{time::Duration, error::Error, fmt::Display, ops::DerefMut};

use serialport::SerialPort;
use soup_icd::ToSoup;


pub fn connect(looking_for: PortKind) -> Result<Box<dyn SerialPort>, Box<dyn Error>> {
    let mut last_err: Option<FindError> = None;

    let port = loop {
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
                crate::send(ToSoup::Control(soup_icd::Control::Reboot), port.deref_mut())?;
            }
            (PortKind::SoupApp, PortKind::Stage0) => {
                println!(" -> Looking for an application, but found a stage0 loader.");
                println!(" -> Cannot continue.");
                println!(" -> Try Loading an app with `soup-cli stage0 ...` commands.");
                return Err("No application found.".into());
            }
        }
    };

    Ok(port)
}

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
        .timeout(Duration::from_millis(16))
        .open()?;

    Ok((*kind, port))
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
