#![no_std]

#[cfg(not(feature = "defmt"))]
/// NOOP log macros when defmt is not enabled
pub(crate) mod log {
    #[allow(unused_macros)]
    #[macro_export]
    macro_rules! debug {
        ($($arg:tt)+) => {};
    }
    #[allow(unused_macros)]
    #[macro_export]
    macro_rules! info {
        ($($arg:tt)+) => {};
    }
    #[allow(unused_macros)]
    #[macro_export]
    macro_rules! warn {
        ($($arg:tt),+) => {};
    }
    #[allow(unused_macros)]
    #[macro_export]
    macro_rules! error {
        ($($arg:tt),+) => {};
    }
}

#[allow(unused_imports)]
#[cfg(feature = "defmt")]
pub(crate) mod log {
    pub use defmt::{debug, error, info, warn};
}

pub(crate) use log::*;

/// Re-export embedded_io::Read because it is used in the public API
pub use embedded_io;

// Make sure at least one part feature is enabled
#[cfg(not(any(
    feature = "stm32g431xb",
    feature = "stm32g431x8",
    feature = "stm32g431x6"
)))]
compile_error!("No chip feature is enabled for rjmp-stm32-flash");

mod dual_page;
pub(crate) mod fletcher16;
pub use dual_page::*;
mod dual_persist;
pub use dual_persist::*;
pub mod flash_gx;
pub use flash_gx::*;
