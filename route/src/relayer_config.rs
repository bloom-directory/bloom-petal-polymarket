use alloy::primitives::Address;
use petal::sdk::{DispatchResponse, HostStatus, SdkError};
use serde::{Deserialize, Serialize};

use crate::constants::MAX_STORE_BYTES;
use crate::infra_parts::util::sdk_error;
use crate::prelude::error;

const RELAYER_CONFIG_KEY: &str = "config/relayer.json";

fn default_builder_key_mode() -> String {
    "auto".into()
}

#[derive(Clone, Deserialize, Serialize)]
pub struct RelayerConfig {
    #[serde(default = "default_builder_key_mode")]
    pub builder_key_mode: String,
    #[serde(default)]
    pub relayer_api_key: Option<String>,
    #[serde(default)]
    pub relayer_api_key_address: Option<String>,
    #[serde(default)]
    pub legacy_eoa_mode: bool,
}

impl core::fmt::Debug for RelayerConfig {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("RelayerConfig")
            .field("builder_key_mode", &self.builder_key_mode)
            .field(
                "relayer_api_key",
                &self.relayer_api_key.as_ref().map(|_| "<redacted>"),
            )
            .field("relayer_api_key_address", &self.relayer_api_key_address)
            .field("legacy_eoa_mode", &self.legacy_eoa_mode)
            .finish()
    }
}

impl Default for RelayerConfig {
    fn default() -> Self {
        Self {
            builder_key_mode: default_builder_key_mode(),
            relayer_api_key: None,
            relayer_api_key_address: None,
            legacy_eoa_mode: false,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum RelayerAuth {
    AutoBuilder,
    Manual { key: String, address: String },
    Disabled { reason: String },
}

impl core::fmt::Debug for RelayerAuth {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::AutoBuilder => formatter.write_str("AutoBuilder"),
            Self::Manual { address, .. } => formatter
                .debug_struct("Manual")
                .field("key", &"<redacted>")
                .field("address", address)
                .finish(),
            Self::Disabled { reason } => formatter
                .debug_struct("Disabled")
                .field("reason", reason)
                .finish(),
        }
    }
}

impl RelayerAuth {
    pub fn label(&self) -> &'static str {
        match self {
            Self::AutoBuilder => "builder_key_auto",
            Self::Manual { .. } => "relayer_key_manual",
            Self::Disabled { .. } => "disabled",
        }
    }
}

pub fn configured_relayer_auth() -> Result<RelayerAuth, DispatchResponse> {
    Ok(load_relayer_config()?.auth())
}

impl RelayerConfig {
    pub fn validate(self) -> Result<Self, String> {
        let mode = self.builder_key_mode.trim().to_ascii_lowercase();
        if !matches!(mode.as_str(), "auto" | "manual" | "disabled") {
            return Err(format!(
                "unknown builder_key_mode '{mode}' (expected auto, manual, or disabled)"
            ));
        }
        let key = self
            .relayer_api_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let address = self
            .relayer_api_key_address
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        match (key, address) {
            (Some(_), Some(address)) => {
                address
                    .parse::<Address>()
                    .map_err(|err| format!("relayer_api_key_address: {err}"))?;
            }
            (None, None) => {}
            _ => {
                return Err(
                    "relayer_api_key and relayer_api_key_address must be configured together"
                        .into(),
                );
            }
        }
        Ok(Self {
            builder_key_mode: mode,
            relayer_api_key: key.map(str::to_owned),
            relayer_api_key_address: address.map(str::to_owned),
            legacy_eoa_mode: self.legacy_eoa_mode,
        })
    }

    pub fn auth(&self) -> RelayerAuth {
        if let (Some(key), Some(address)) = (
            self.relayer_api_key.as_ref(),
            self.relayer_api_key_address.as_ref(),
        ) {
            return RelayerAuth::Manual {
                key: key.clone(),
                address: address.clone(),
            };
        }
        match self.builder_key_mode.as_str() {
            "auto" => RelayerAuth::AutoBuilder,
            "manual" => RelayerAuth::Disabled {
                reason: "builder_key_mode = \"manual\" but relayer_api_key / relayer_api_key_address are not configured"
                    .into(),
            },
            "disabled" => RelayerAuth::Disabled {
                reason: "builder_key_mode = \"disabled\": relayer auth is off, so deposit-wallet operations are unavailable"
                    .into(),
            },
            other => RelayerAuth::Disabled {
                reason: format!(
                    "unknown builder_key_mode '{other}' (expected auto, manual, or disabled)"
                ),
            },
        }
    }

    pub fn public_json(&self) -> serde_json::Value {
        serde_json::json!({
            "builder_key_mode": self.builder_key_mode,
            "manual_credentials_configured": self.relayer_api_key.is_some()
                && self.relayer_api_key_address.is_some(),
            "relayer_api_key": if self.relayer_api_key.is_some() { "<redacted>" } else { "" },
            "relayer_api_key_address": self.relayer_api_key_address,
            "legacy_eoa_mode": self.legacy_eoa_mode,
            "trading_mode": if self.legacy_eoa_mode { "credentials_read_only" } else { "deposit_wallet_v2" },
        })
    }
}

pub fn load_relayer_config() -> Result<RelayerConfig, DispatchResponse> {
    match petal::sdk::store_get(RELAYER_CONFIG_KEY, MAX_STORE_BYTES) {
        Ok(bytes) => serde_json::from_slice::<RelayerConfig>(&bytes)
            .map_err(|err| error(-4, format!("corrupt relayer settings: {err}")))?
            .validate()
            .map_err(|message| error(-4, format!("invalid relayer settings: {message}"))),
        Err(SdkError::Host(HostStatus::NotFound)) => Ok(RelayerConfig::default()),
        Err(err) => Err(sdk_error(err)),
    }
}

pub fn read_relayer_config() -> DispatchResponse {
    match load_relayer_config() {
        Ok(config) => petal::read_json_value(&config.public_json()),
        Err(resp) => resp,
    }
}

pub fn write_relayer_config(body: &[u8]) -> DispatchResponse {
    let config = match serde_json::from_slice::<RelayerConfig>(body) {
        Ok(config) => match config.validate() {
            Ok(config) => config,
            Err(message) => return error(-3, message),
        },
        Err(err) => return error(-3, format!("relayer settings JSON: {err}")),
    };
    let bytes = match serde_json::to_vec_pretty(&config) {
        Ok(bytes) => bytes,
        Err(err) => return error(-4, format!("relayer settings JSON: {err}")),
    };
    match petal::sdk::store_put(RELAYER_CONFIG_KEY, &bytes, true) {
        Ok(()) => DispatchResponse::Write,
        Err(err) => sdk_error(err),
    }
}

pub fn require_v2_trading() -> Result<(), DispatchResponse> {
    require_v2_trading_config(&load_relayer_config()?)
}

pub fn legacy_eoa_status(wallet: &str, owner: Address) -> serde_json::Value {
    serde_json::json!({
        "wallet": wallet,
        "owner": owner.to_checksum(None),
        "mode": "credentials_read_only",
        "stage": "legacy_eoa",
        "running": false,
        "tradeable": false,
        "creds_present": true,
        "deposit_wallet": serde_json::Value::Null,
        "approvals": {
            "required": false,
        },
        "probes": {
            "source": "owner_eoa",
        },
        "message": "legacy EOA compatibility stores CLOB credentials for reads only; V2 trading and value-moving operations require deposit-wallet mode",
    })
}

fn require_v2_trading_config(config: &RelayerConfig) -> Result<(), DispatchResponse> {
    if config.legacy_eoa_mode {
        return Err(error(
            -3,
            "legacy_eoa_mode is credentials/read-only compatibility; V2 trading and value-moving operations require deposit-wallet mode",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(mode: &str, key: Option<&str>, address: Option<&str>) -> RelayerConfig {
        RelayerConfig {
            builder_key_mode: mode.into(),
            relayer_api_key: key.map(str::to_owned),
            relayer_api_key_address: address.map(str::to_owned),
            legacy_eoa_mode: false,
        }
    }

    #[test]
    fn manual_pair_wins_and_partial_pair_fails_closed() {
        let address = "0xE51282BdEeeb988406B3f969a6277b02bAdc2e19";
        let manual = config("disabled", Some("secret-key"), Some(address))
            .validate()
            .unwrap();
        assert_eq!(
            manual.auth(),
            RelayerAuth::Manual {
                key: "secret-key".into(),
                address: address.into(),
            }
        );
        assert!(
            config("manual", Some("secret-key"), None)
                .validate()
                .is_err()
        );
        assert!(config("manual", None, Some(address)).validate().is_err());
    }

    #[test]
    fn modes_fail_closed_and_public_view_redacts_secret() {
        assert_eq!(
            config("auto", None, None).validate().unwrap().auth(),
            RelayerAuth::AutoBuilder
        );
        for mode in ["manual", "disabled"] {
            assert!(matches!(
                config(mode, None, None).validate().unwrap().auth(),
                RelayerAuth::Disabled { .. }
            ));
        }
        assert!(config("surprise", None, None).validate().is_err());

        let manual = config(
            "manual",
            Some("must-not-leak"),
            Some("0xE51282BdEeeb988406B3f969a6277b02bAdc2e19"),
        )
        .validate()
        .unwrap();
        let public = manual.public_json().to_string();
        assert!(!public.contains("must-not-leak"));
        assert!(public.contains("<redacted>"));
        assert!(!format!("{manual:?}").contains("must-not-leak"));
        assert!(!format!("{:?}", manual.auth()).contains("must-not-leak"));
    }

    #[test]
    fn legacy_mode_common_guard_rejects_value_moving_entry_points() {
        let mut config = RelayerConfig::default();
        assert!(require_v2_trading_config(&config).is_ok());
        config.legacy_eoa_mode = true;
        let error = require_v2_trading_config(&config).unwrap_err();
        assert!(matches!(
            error,
            DispatchResponse::Error { code: -3, message }
                if message.contains("credentials/read-only")
        ));
        let status = legacy_eoa_status(
            "alice",
            "0xE51282BdEeeb988406B3f969a6277b02bAdc2e19"
                .parse()
                .unwrap(),
        );
        assert_eq!(status["tradeable"], false);
        assert_eq!(status["mode"], "credentials_read_only");
        assert!(status["deposit_wallet"].is_null());
    }
}
