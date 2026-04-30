use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EnsRecord {
    pub address: String,
    pub name: Option<String>,
    pub twitter: Option<String>,
    pub github: Option<String>,
    pub telegram: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EnsSubgraph {
    endpoint: String,
}

impl EnsSubgraph {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub async fn lookup(&self, _address: &str) -> Result<Option<EnsRecord>> {
        Ok(None)
    }
}
