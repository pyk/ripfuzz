//! Campaign result aggregation.

use crate::corpus::CoverageMap;
use crate::worker::PropertyFailure;

/// The aggregated output of a fuzzing campaign.
#[derive(Debug)]
pub struct CampaignResult {
    pub runs: u64,
    pub failures: Vec<PropertyFailure>,
    /// Total individual calls executed across all workers.
    pub total_calls: u64,
    /// Total gas consumed across all calls.
    pub total_gas: u64,
    /// Actual wall-clock elapsed time in seconds.
    pub elapsed_secs: f64,
    /// Final global coverage map after all workers finish.
    pub coverage: CoverageMap,
}
