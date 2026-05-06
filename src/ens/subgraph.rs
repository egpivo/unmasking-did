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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_and_endpoint() {
        let g = EnsSubgraph::new("https://example.com/subgraph");
        assert_eq!(g.endpoint(), "https://example.com/subgraph");
    }

    #[tokio::test]
    async fn lookup_returns_none_stub() {
        let g = EnsSubgraph::new("https://example.com/subgraph");
        let r = g.lookup("0x0000000000000000000000000000000000000001").await;
        assert!(r.expect("ok").is_none());
    }
}
