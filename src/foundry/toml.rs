use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct FoundryToml {
    pub profile: HashMap<String, FoundryProfile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FoundryProfile {
    pub src: Option<String>,
    pub out: Option<String>,
    pub test: Option<String>,
    pub script: Option<String>,
}

impl FoundryToml {
    pub fn default_profile(&self) -> &FoundryProfile {
        self.profile
            .get("default")
            .or_else(|| self.profile.get("$default"))
            .unwrap_or_else(|| {
                self.profile
                    .values()
                    .next()
                    .expect("foundry.toml has at least one profile")
            })
    }
}

impl FoundryProfile {
    pub fn out(&self) -> &str {
        self.out.as_deref().unwrap_or("out")
    }

    pub fn src(&self) -> &str {
        self.src.as_deref().unwrap_or("src")
    }

    pub fn test(&self) -> &str {
        self.test.as_deref().unwrap_or("test")
    }
}
