//! A fancy container for owned or borrowed vecs
//!
//! So, sometimes you want to use owned types, and have the std
//! library. And other times you don't, and borrowed types are
//! okay. This handles both cases, based on a feature flag.
//!
//! Inspired by @whitequark's `managed` crate.
//! Stolen again from postcard-infomem

use core::fmt::Debug;
use serde::{Deserialize, Serialize};

#[cfg(feature = "use-std")]
use std::vec::Vec;

#[cfg_attr(feature = "use-defmt", derive(defmt::Format))]
pub enum Managed<'a> {
    /// Borrowed variant.
    Borrowed(&'a [u8]),
    #[cfg(feature = "use-std")]
    /// Owned variant, only available with the std or alloc feature enabled.
    Owned(Vec<u8>),
}

impl<'a> Managed<'a> {
    /// Create an Managed from a borrowed slice
    pub fn from_borrowed(s: &'a [u8]) -> Self {
        Managed::Borrowed(s)
    }

    /// Create an Managed from an owned Vec
    #[cfg(feature = "use-std")]
    pub fn from_vec(v: Vec<u8>) -> Managed<'static> {
        Managed::Owned(v)
    }

    /// View the Managed as a slice
    pub fn as_slice(&'a self) -> &'a [u8] {
        match self {
            Managed::Borrowed(s) => s,
            #[cfg(feature = "use-std")]
            Managed::Owned(s) => s.as_slice(),
        }
    }

    #[cfg(feature = "use-std")]
    pub fn to_owned(&'a self) -> Managed<'static> {
        match self {
            Managed::Borrowed(b) => Managed::Owned(b.to_vec()),
            Managed::Owned(s) => Managed::Owned(s.clone()),
        }
    }
}

// Optional impls

#[cfg(feature = "use-std")]
impl From<Vec<u8>> for Managed<'static> {
    fn from(s: Vec<u8>) -> Self {
        Managed::Owned(s)
    }
}

#[cfg(feature = "use-std")]
impl From<Managed<'static>> for Vec<u8> {
    fn from(is: Managed<'static>) -> Self {
        match is {
            Managed::Borrowed(s) => s.to_vec(),
            Managed::Owned(s) => s,
        }
    }
}

// Implement a couple traits by passing through to &[u8]'s methods

impl<'a> From<&'a [u8]> for Managed<'a> {
    fn from(s: &'a [u8]) -> Self {
        Managed::Borrowed(s)
    }
}

impl<'a> From<&'a Managed<'a>> for &'a [u8] {
    fn from(is: &'a Managed<'a>) -> &'a [u8] {
        is.as_slice()
    }
}

impl<'a> PartialEq for Managed<'a> {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice().eq(other.as_slice())
    }
}

impl<'a> Debug for Managed<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.as_slice().fmt(f)
    }
}

impl<'a> Serialize for Managed<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.as_slice().serialize(serializer)
    }
}

impl<'a, 'de: 'a> Deserialize<'de> for Managed<'a> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = <&'de [u8] as Deserialize<'de>>::deserialize(deserializer)?;
        Ok(Managed::Borrowed(s))
    }
}
