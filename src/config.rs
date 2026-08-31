//! `config.toml` — one `[[profile]]` per household member. Deliberately
//! carries nothing identifying: `name` is just a local label choosing which
//! `data/{name}/...` subtree a person's receipts land in — BankID logins
//! for both ICA (via Kivra) and Willys are pure signature-based, no
//! personnummer ever needed here.
//!
//! Ported from `khessman/ica-sync`'s `config.rs`, targeting
//! `kvitto_core::ProfileId` directly instead of a bespoke `Profile` struct
//! (that concept already lives in core now).

use kvitto_core::ProfileId;
use serde::Deserialize;

#[derive(Deserialize, Default)]
struct RawConfig {
    #[serde(default)]
    profile: Vec<RawProfile>,
}

#[derive(Deserialize)]
struct RawProfile {
    name: String,
}

/// Reads `config.toml` if present. Not checked in (see `.gitignore`) — it's
/// private per household/clone. Missing entirely is not an error: falls
/// back to a single synthesized profile so the tool still works before
/// anyone's set up multi-person config.
pub fn load_profiles(path: &str) -> anyhow::Result<Vec<ProfileId>> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(vec![ProfileId("du".to_string())])
        }
        Err(e) => return Err(e).map_err(|e| anyhow::anyhow!("could not read {path}: {e}")),
    };
    let parsed: RawConfig =
        toml::from_str(&raw).map_err(|e| anyhow::anyhow!("could not parse {path} as TOML: {e}"))?;

    if parsed.profile.is_empty() {
        return Ok(vec![ProfileId("du".to_string())]);
    }
    Ok(parsed.profile.into_iter().map(|p| ProfileId(p.name)).collect())
}
