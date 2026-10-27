use std::{fs, net::Ipv4Addr, path::Path};

use serde::{Deserialize, Serialize};

use crate::{error, Result};

pub const STATE: &str = "/var/lib/orby";
pub const MACHINE: &str = "orby";

// Preserve the format written by the Go-era activation, including the TTL in
// seconds. The root-owned file is also needed to safely disable older installs.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RunnerConfig {
    pub orbctl: String,
    pub machine: String,
    pub port: u16,
    #[serde(rename = "on-demand")]
    pub on_demand: bool,
    pub ttl: u64,
}

impl RunnerConfig {
    pub fn read(path: &Path) -> Result<Self> {
        let config: Self = serde_json::from_slice(&fs::read(path)?)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if !Path::new(&self.orbctl).is_absolute()
            || self.orbctl.contains('\n')
            || self.machine != MACHINE
            || self.port < 1024
            || self.ttl == 0
        {
            return Err(error("invalid Orby runner configuration"));
        }
        Ok(())
    }
}

pub fn first_ipv4(output: &str) -> Result<Ipv4Addr> {
    output
        .split_whitespace()
        .find_map(|part| part.parse().ok())
        .ok_or_else(|| error("no IPv4 address returned by Orby machine"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_runner_config_is_supported() {
        let config: RunnerConfig = serde_json::from_str(
            r#"{"orbctl":"/opt/homebrew/bin/orbctl","machine":"orby","port":31222,"on-demand":true,"ttl":900}"#,
        )
        .unwrap();
        config.validate().unwrap();
        assert!(config.on_demand);
        assert_eq!(config.ttl, 900);
        assert_eq!(
            first_ipv4("fd07::1 198.19.1.5").unwrap().to_string(),
            "198.19.1.5"
        );
        for bad in ["", "orbctl"] {
            let mut invalid = config.clone();
            invalid.orbctl = bad.into();
            assert!(invalid.validate().is_err());
        }
        let mut invalid = config;
        invalid.machine = "another-machine".into();
        assert!(invalid.validate().is_err());
        assert!(first_ipv4("fd07::1").is_err());
    }
}
