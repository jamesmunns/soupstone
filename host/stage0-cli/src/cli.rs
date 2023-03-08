use std::{num::ParseIntError, str::FromStr};
use clap::{Args, Parser, Subcommand};
// use phm::Machine;

#[derive(Debug)]
pub struct Address(pub u32);

#[derive(Debug)]
pub struct WriteBytes(pub Vec<u8>);

#[derive(Parser, Debug)]
pub enum Stage0 {
    /// Peek
    Peek(Peek),
    /// Poke
    Poke(Poke),
}

#[derive(Parser, Debug)]
pub struct Peek {
    #[clap(subcommand)]
    pub command: PeekCommand,
}

#[derive(Parser, Debug)]
pub struct Poke {
    #[clap(subcommand)]
    pub command: PokeCommand,
}

#[derive(Subcommand, Debug)]
pub enum PeekCommand {
    /// Write bytes to the given address
    #[clap(name = "u8")]
    PeekU8(PeekU8),
}

#[derive(Subcommand, Debug)]
pub enum PokeCommand {
    /// Write bytes
    #[clap(name = "u8")]
    PokeU8(PokeU8),
}

#[derive(Args, Debug)]
pub struct PeekU8 {
    /// The address to write to.
    #[clap(short = 'a')]
    pub address: Address,
}

#[derive(Args, Debug)]
pub struct PokeU8 {
    /// The address to write to.
    #[clap(short = 'a')]
    pub address: Address,
    /// Bytes to write to the address. Should be given as a comma-separated list of hex values. For example: "0xA0,0xAB,0x11".
    #[clap(short = 'b', long = "write")]
    pub val: WriteBytes,
}

// #[derive(Args, Debug)]
// struct WriteRead {
//     /// The address to write to. Should be given as a hex value. For example: "0xA4".
//     #[clap(short = 'a')]
//     address: Address,
//     #[clap(short = 'b', long = "bytes")]
//     write_bytes: WriteBytes,
//     /// Bytes to write to the address. Should be given as a comma-separated list of hex values. For example: "0xA0,0xAB,0x11".
//     #[clap(long = "read-ct")]
//     read_count: usize,
// }

// #[derive(Args, Debug)]
// struct SpiWrite {
//     /// Bytes to write over SPI. Should be given as a comma-separated list of hex values. For example: "0xA0,0xAB,0x11".
//     #[clap(short = 'b', long = "write")]
//     write_bytes: WriteBytes,
// }

// #[derive(Args, Debug)]
// struct SpiTransfer {
//     /// Bytes to transfer over SPI. Should be given as a comma-separated list of hex values. For example: "0xA0,0xAB,0x11".
//     #[clap(short = 'b', long = "write")]
//     write_bytes: WriteBytes,
// }

// impl Stage0 {
//     pub fn run(&self, machine: &mut Machine) -> Result<String, phm::Error> {
//         match self {
//             Stage0::I2C(cmd) => match &cmd.command {
//                 I2CCommand::I2CWrite(args) => embedded_hal::blocking::i2c::Write::write(
//                     machine,
//                     args.address.0,
//                     &args.write_bytes.0,
//                 )
//                 .map(|_| "".into()),
//                 I2CCommand::I2CRead(args) => {
//                     let mut buffer = vec![0u8; args.read_count];
//                     embedded_hal::blocking::i2c::Read::read(machine, args.address.0, &mut buffer)?;
//                     Ok(format!("{:02x?}", &buffer))
//                 }
//                 I2CCommand::WriteRead(args) => {
//                     let mut buffer = vec![0u8; args.read_count];
//                     embedded_hal::blocking::i2c::WriteRead::write_read(
//                         machine,
//                         args.address.0,
//                         &args.write_bytes.0,
//                         &mut buffer,
//                     )?;
//                     Ok(format!("{:02x?}", &buffer))
//                 }
//             },
//             Stage0::Spi(cmd) => match &cmd.command {
//                 SpiCommand::SpiWrite(args) => {
//                     embedded_hal::blocking::spi::Write::write(machine, &args.write_bytes.0)
//                         .map(|_| "".into())
//                 }
//                 SpiCommand::SpiTransfer(args) => {
//                     let mut buffer = args.write_bytes.0.clone();
//                     embedded_hal::blocking::spi::Transfer::transfer(machine, &mut buffer)
//                         .map(|bytes| format!("{:02x?}", &bytes))
//                 }
//             },
//         }
//     }
// }

impl FromStr for WriteBytes {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut bytes: Vec<u8> = Vec::new();
        for b in s.split(',') {
            let without_prefix = b.trim_start_matches("0x");
            let byte = u8::from_str_radix(without_prefix, 16)?;
            bytes.push(byte);
        }

        Ok(Self(bytes))
    }
}

impl FromStr for Address {
    type Err = ParseIntError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let without_prefix = s.trim_start_matches("0x");
        let byte = u32::from_str_radix(without_prefix, 16)?;

        Ok(Self(byte))
    }
}
