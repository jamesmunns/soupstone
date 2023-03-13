#![cfg_attr(not(any(test, feature = "use-std")), no_std)]

pub use soup_managed::Managed;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum ToSoup<'a> {
    #[serde(borrow)]
    Stdin(Managed<'a>),
    Control(Control),
    ToApp(Managed<'a>),
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum FromSoup<'a> {
    #[serde(borrow)]
    Stdout(Managed<'a>),
    Stderr(Managed<'a>),
    ControlResponse(ControlResponse<'a>),
    FromApp(Managed<'a>),
    Error(Error<'a>),
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Error<'a> {
    #[serde(borrow)]
    Other(Managed<'a>),
    InvalidMessage,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Control {
    Reboot,
    SendAppInfo,
}

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum ControlResponse<'a> {
    #[serde(borrow)]
    AppInfo(Managed<'a>),
}
