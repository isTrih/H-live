//! Native H-live collection, lifecycle and desktop request interface.
mod maintenance;
mod community;
mod clips;
mod clip_history;
mod clip_naming;
mod session_marks;
mod anchor_tags;
mod time_display;
pub mod data_backup;
mod clip_plan;
mod clip_worker;
pub use community::{CommunityReplayLookup, validated_media_url};
mod monitor;
pub mod net;
mod polling;
pub mod proxy;
mod requests;
mod highlights;
pub use highlights::HighlightModelDiscovery;
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
