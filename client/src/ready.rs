use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReadyState {
    pub schema_version: u32,
    pub pid: u32,
    pub api_origin: String,
}

impl ReadyState {
    pub fn parse(data: &[u8]) -> Result<Self> {
        let state: Self = serde_json::from_slice(data).context("decode API ready state")?;
        state.validate()?;
        Ok(state)
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1 || self.pid == 0 {
            bail!("invalid API ready state");
        }
        let address = self
            .api_origin
            .strip_prefix("http://")
            .context("API origin must use http")?
            .parse::<std::net::SocketAddr>()
            .context("parse API origin")?;
        if !address.ip().is_loopback() || address.port() == 0 {
            bail!("API origin must use a loopback address and assigned port");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_loopback_ready_origins() {
        let valid = br#"{"schema_version":1,"pid":42,"api_origin":"http://127.0.0.1:49152"}"#;
        assert!(ReadyState::parse(valid).is_ok());
        let foreign = br#"{"schema_version":1,"pid":42,"api_origin":"http://192.168.1.2:49152"}"#;
        assert!(ReadyState::parse(foreign).is_err());
        let unassigned = br#"{"schema_version":1,"pid":42,"api_origin":"http://127.0.0.1:0"}"#;
        assert!(ReadyState::parse(unassigned).is_err());
    }
}
