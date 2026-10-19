//! Native H-live collection, lifecycle and desktop request interface.
mod maintenance;
mod monitor;
pub mod net;
mod polling;
pub mod proxy;
mod requests;
mod runtime;
pub mod signature;
pub mod storage;
pub mod transport;
pub use runtime::{Runtime, VERSION};

mod self_test;
pub use self_test::run as offline_self_test;
#[cfg(feature = "test-support")]
pub mod testing;

pub mod process;
#[cfg(test)]
mod runtime_tests;

#[cfg(test)]
mod transport_tests;
