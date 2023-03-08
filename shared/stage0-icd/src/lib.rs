#![cfg_attr(not(any(test, feature = "use-std")), no_std)]

use managedbuf::Managed;
use serde::{Deserialize, Serialize};

pub mod managedbuf;

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Request<'a> {
    // Peek
    PeekU8 {
        addr: usize,
    },
    PeekU16 {
        addr: usize,
    },
    PeekU32 {
        addr: usize,
    },
    PeekBytes {
        addr: usize,
        len: usize,
    },

    // Poke
    PokeU8 {
        addr: usize,
        val: u8,
    },
    PokeU16 {
        addr: usize,
        val: u16,
    },
    PokeU32 {
        addr: usize,
        val: u32,
    },
    PokeBytes {
        addr: usize,
        #[serde(borrow)]
        val: Managed<'a>,
    },

    // Other
    ClearMagic,
    Reboot,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Error {
    AddressOutOfRange {
        request: usize,
        min: usize,
        max: usize,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Response<'a> {
    PeekU8 {
        addr: usize,
        val: u8,
    },
    PeekU16 {
        addr: usize,
        val: u16,
    },
    PeekU32 {
        addr: usize,
        val: u32,
    },
    PeekBytes {
        addr: usize,
        #[serde(borrow)]
        val: Managed<'a>,
    },
    Poked {
        addr: usize,
    },
    MagicCleared,
}
