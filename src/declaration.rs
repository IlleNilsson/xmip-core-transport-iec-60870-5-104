//! This technology's declaration: the transport guide's "The declaration",
//! `module/core/capability/transport/doc/adding-a-transport.md`.

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

/// This technology's schemes, as [`transport::Configured::SCHEMES`] defines
/// them.
pub const SCHEMES: &[&str] = &["iec104"];
