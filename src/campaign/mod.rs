//! Campaign orchestration: configuration, setup, and result aggregation.

pub use config::CampaignConfig;
pub use core::{Campaign, CampaignBuilder};
pub use result::CampaignResult;
pub use seeds::build_seeds;

pub mod config;
pub mod core;
pub mod result;
pub mod seeds;
