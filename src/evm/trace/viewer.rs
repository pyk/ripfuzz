use super::Trace;

/// Enriched trace viewer that adds labels, formats output, and builds a
/// callgraph from raw [`Trace`] data.
#[derive(Debug, Clone, Default)]
pub struct Viewer;

impl Viewer {
    /// Create a new viewer.
    pub fn new() -> Self {
        Self
    }

    /// Enrich a raw trace with labels and return a formatted representation.
    pub fn view(&self, _trace: &Trace) -> String {
        // Placeholder: will be implemented later.
        String::new()
    }
}
