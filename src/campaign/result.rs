//! Campaign result aggregation.

use crate::corpus::CoverageMap;
use crate::worker::PropertyFailure;

/// The aggregated output of a fuzzing campaign.
#[derive(Debug)]
pub struct CampaignResult {
    pub runs: u64,
    pub failures: Vec<PropertyFailure>,
    /// Final global coverage map after all workers finish.
    pub coverage: CoverageMap,
}
