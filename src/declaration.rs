//! What a Location of this technology is configured with: its declaration,
//! which `Configured` reads, built with the crate's `declaration` feature
//! alone and the code behind the default (ADR-0064, amendments 2026-10-22
//! and 2026-11-26), for the library the surfaces load to carry (open
//! problem 48).

use xcore::settings::{Applies, Settings};

/// Every setting a Location takes beyond its address, where a Receive
/// Location listens as the controlled station, `0.0.0.0:2404` by the
/// standard; a Send Location connects to the station its target names.
pub const SETTINGS: &Settings = &Settings {
    technology: env!("CARGO_PKG_NAME"),
    settings: &[transport::configured::timeout(
        Applies::Both,
        None,
        "How long a connection is waited for, and a peer that stops mid-APDU \
         is waited on; unbounded when left out.",
    )],
};

/// The schemes a Location's address is entered with to choose this technology.
pub const SCHEMES: &[&str] = &["iec104"];
