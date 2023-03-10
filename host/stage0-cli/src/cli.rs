use std::{num::ParseIntError, str::FromStr};
use clap::{Args, Parser};

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
    /// Bootload
    Bootload(Bootload)
}

#[derive(Args, Debug)]
pub struct Peek {
    /// The address to read from.
    #[clap(short = 'a')]
    pub address: Address,

    /// How many bytes to read
    #[clap(short = 'l', long = "count")]
    pub count: usize,

    /// Output File. Prints to stdout if not provided
    #[clap(short = 'f', long = "file")]
    pub file: Option<String>,
}

#[derive(Args, Debug)]
pub struct Poke {
    /// The address to write to.
    #[clap(short = 'a')]
    pub address: Address,
    /// Bytes to write to the address. For example: "0xA0,0xAB,0x11".
    #[clap(short = 'b', long = "write")]
    pub val: Option<WriteBytes>,

    /// Input file
    #[clap(short = 'f', long = "file")]
    pub file: Option<String>,
}

#[derive(Args, Debug)]
pub struct Bootload {
    /// The address to write to.
    #[clap(short = 'a')]
    pub address: Address,
}

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
