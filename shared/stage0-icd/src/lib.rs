#![cfg_attr(not(any(test, feature = "use-std")), no_std)]

pub use soup_managed::Managed;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Request<'a> {
    // Peek
    PeekBytes {
        addr: usize,
        len: usize,
    },

    // Poke
    PokeBytes {
        addr: usize,
        #[serde(borrow)]
        val: Managed<'a>,
    },

    // Other
    ClearMagic,
    Reboot,
    Bootload {
        addr: u32,
    },

    // Flash
    PeekBytesFlash {
        addr: usize,
        len: usize,
    },
    FlashCopy {
        ram_start: usize,
        flash_start: usize,
        len: usize,
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Error {
    AddressOutOfRange {
        request: usize,
        len: usize,
        min: usize,
        max: usize,
    },
    RangeTooLarge {
        request: usize,
        max: usize,
    },
    UnalignedFlashAddr(UnalignedFlashAddr),
    CantOverwriteBootloader,
    FlashCopyFailed,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub struct PeekBytes<'a> {
    pub addr: usize,
    #[serde(borrow)]
    pub val: Managed<'a>,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub struct Poked {
    pub addr: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub struct UnalignedFlashAddr{
    pub addr: usize,
    pub align: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Response<'a> {
    #[serde(borrow)]
    PeekBytes(PeekBytes<'a>),
    Poked(Poked),
    MagicCleared,
    #[serde(borrow)]
    PeekBytesFlash(PeekBytes<'a>),
    FlashCopied,
}

#[cfg(feature = "use-std")]
impl<'a> PeekBytes<'a> {
    pub fn to_owned(&self) -> PeekBytes<'static> {
        PeekBytes { addr: self.addr, val: self.val.to_owned() }
    }
}

#[cfg(feature = "use-std")]
impl<'a> Response<'a> {
    pub fn to_owned(&self) -> Response<'static> {
        match self {
            Response::PeekBytes(pb) => Response::PeekBytes(pb.to_owned()),
            Response::PeekBytesFlash(pbf) => Response::PeekBytesFlash(pbf.to_owned()),
            Response::Poked(Poked { addr }) => Response::Poked(Poked { addr: *addr }),
            Response::MagicCleared => Response::MagicCleared,
            Response::FlashCopied => Response::FlashCopied,
        }
    }
}
