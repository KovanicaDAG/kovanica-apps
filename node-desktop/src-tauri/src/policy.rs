//! Checks a selected `kovanica-node` config before the process is spawned.
//!
//! Semantics match `NodeConfig` defaults and CLI behavior reviewed in
//! KovanicaDAG/kovanica `74045cf3e27a70569c32af35aec24ada5dd9ff79`:
//! `kovanica-node --config <file>` (no `--data-dir` flag). `data_dir` lives in
//! the file. `run_validator` defaults to true. `mining.enabled` defaults to
//! false. `chain_id` defaults to 1337. `rpc_addr` defaults to `127.0.0.1`
//! and `rpc_port` to 8545. `KOVANICA_*` environment variables override the
//! file inside the node, so this supervisor refuses to start while any are set.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

pub const REVIEWED_NODE_SOURCE: &str = "74045cf3e27a70569c32af35aec24ada5dd9ff79";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckedSetup {
    pub binary: String,
    pub binary_sha256: String,
    pub config_path: String,
    pub data_dir: String,
    pub chain_id: u64,
    pub chain_label: String,
    pub chain_id_from_file: bool,
    pub rpc_addr: String,
    pub rpc_port: u16,
    pub rpc_http: String,
    pub listen_addr: String,
    pub mining_enabled: bool,
    pub run_validator: bool,
}

pub fn chain_label(chain_id: u64) -> Result<&'static str, String> {
    match chain_id {
        1 => Err(
            "chain id 1 is mainnet. This milestone cannot start mainnet, and it will not relabel the network."
                .into(),
        ),
        2 => Ok("legacy testnet (chain 2)"),
        3 => Ok("devnet (chain 3)"),
        4 => Ok("testnet v2 (chain 4)"),
        5 => Ok("hybrid testnet (chain 5)"),
        1337 => Ok("local (chain 1337)"),
        other => Err(format!(
            "chain id {other} is not a supported development network in this milestone (allowed: 2, 3, 4, 5, 1337). It was not relabelled."
        )),
    }
}

/// Names of `KOVANICA_*` variables present in the environment. Values are never returned.
pub fn kovanica_env_names<'a, I>(vars: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut names: Vec<String> = vars
        .into_iter()
        .filter(|name| name.starts_with("KOVANICA_"))
        .map(|name| name.to_string())
        .collect();
    names.sort();
    names.dedup();
    names
}

pub fn check_setup(
    binary: &Path,
    config_path: &Path,
    selected_data_dir: &Path,
    env_names: &[String],
) -> Result<CheckedSetup, String> {
    if !env_names.is_empty() {
        return Err(format!(
            "refusing to start: {} would override the selected config file. Unset them and try again. Values are not shown.",
            env_names.join(", ")
        ));
    }
    let binary = require_absolute_file(binary, "node executable")?;
    if !is_executable(&binary) {
        return Err(format!(
            "{} is not executable. Pass the absolute path of a kovanica-node binary. This app does not search PATH, download, or build one.",
            binary.display()
        ));
    }
    let config_path = require_absolute_file(config_path, "config file")?;
    if !selected_data_dir.is_absolute() {
        return Err(
            "data directory must be an absolute path. The node resolves a relative data_dir against its working directory, which this app will not guess."
                .into(),
        );
    }
    let text = fs::read_to_string(&config_path)
        .map_err(|err| format!("could not read config {}: {err}", config_path.display()))?;
    let value: toml::Value = text
        .parse()
        .map_err(|err| format!("config is not valid TOML: {err}"))?;
    let table = value.as_table().ok_or("config root must be a TOML table")?;

    let (chain_id, chain_id_from_file) = match table.get("chain_id") {
        None => (1337_u64, false),
        Some(toml::Value::Integer(id)) if *id >= 0 => (*id as u64, true),
        Some(_) => {
            return Err(
                "chain_id must be a non-negative integer. This app will not guess or relabel it."
                    .into(),
            );
        }
    };
    let chain_label = chain_label(chain_id)?.to_string();

    let run_validator = match table.get("run_validator") {
        None => {
            return Err(
                "run_validator is not set. The node defaults it to true (validator mode). Set run_validator = false. This app will not rewrite the file."
                    .into(),
            );
        }
        Some(toml::Value::Boolean(value)) => *value,
        Some(_) => return Err("run_validator must be a boolean.".into()),
    };
    if run_validator {
        return Err(
            "run_validator = true. This milestone only supervises a non-validator node. Set run_validator = false. The file was not changed."
                .into(),
        );
    }

    let mining_enabled = mining_enabled(table.get("mining"))?;
    if mining_enabled {
        return Err(
            "mining is enabled. This milestone does not start a miner. Set [mining] enabled = false, or omit the mining table (the node default is off). The file was not changed."
                .into(),
        );
    }

    let rpc_addr = match table.get("rpc_addr") {
        None => "127.0.0.1".to_string(),
        Some(toml::Value::String(value)) => value.clone(),
        Some(_) => return Err("rpc_addr must be a string.".into()),
    };
    if !is_loopback_host(&rpc_addr) {
        return Err(format!(
            "rpc_addr {rpc_addr:?} is not a loopback address. This milestone only accepts 127.0.0.1, localhost, or ::1. The file was not changed."
        ));
    }
    let rpc_port = match table.get("rpc_port") {
        None => 8545_u16,
        Some(toml::Value::Integer(port)) if (1..65536).contains(port) => *port as u16,
        Some(_) => {
            return Err("rpc_port must be an integer from 1 to 65535.".into());
        }
    };

    let configured_data_dir = match table.get("data_dir") {
        Some(toml::Value::String(value)) => PathBuf::from(value),
        None => {
            return Err(
                "data_dir is not set. The node would use a relative ./data path. Set data_dir to the absolute directory you selected. The file was not changed."
                    .into(),
            );
        }
        Some(_) => return Err("data_dir must be a string.".into()),
    };
    if !configured_data_dir.is_absolute() {
        return Err(format!(
            "data_dir {} is relative. Set an absolute path so it does not depend on the working directory. The file was not changed.",
            configured_data_dir.display()
        ));
    }
    let selected = normalize_dir(selected_data_dir)?;
    let configured = normalize_dir(&configured_data_dir)?;
    if selected != configured {
        return Err(format!(
            "selected data directory {} does not match config data_dir {}. The file was not changed and no database was deleted.",
            selected.display(),
            configured.display()
        ));
    }

    let listen_addr = match table.get("listen_addr") {
        None => "0.0.0.0:8000".to_string(),
        Some(toml::Value::String(value)) => value.clone(),
        Some(_) => return Err("listen_addr must be a string.".into()),
    };

    let binary_sha256 = sha256_file(&binary)?;
    let rpc_http = rpc_origin(&rpc_addr, rpc_port);
    Ok(CheckedSetup {
        binary: binary.display().to_string(),
        binary_sha256,
        config_path: config_path.display().to_string(),
        data_dir: configured.display().to_string(),
        chain_id,
        chain_label,
        chain_id_from_file,
        rpc_addr,
        rpc_port,
        rpc_http,
        listen_addr,
        mining_enabled: false,
        run_validator: false,
    })
}

fn mining_enabled(value: Option<&toml::Value>) -> Result<bool, String> {
    match value {
        None => Ok(false),
        Some(toml::Value::Table(table)) => match table.get("enabled") {
            None => Ok(false),
            Some(toml::Value::Boolean(enabled)) => Ok(*enabled),
            Some(_) => Err("mining.enabled must be a boolean.".into()),
        },
        Some(_) => Err("mining must be a TOML table.".into()),
    }
}

pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    let host = host.strip_prefix('[').unwrap_or(host);
    let host = host.strip_suffix(']').unwrap_or(host);
    matches!(
        host.to_ascii_lowercase().as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

pub fn rpc_origin(host: &str, port: u16) -> String {
    if host.contains(':') {
        format!("http://[{host}]:{port}")
    } else {
        format!("http://{host}:{port}")
    }
}

fn require_absolute_file(path: &Path, what: &str) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return Err(format!("choose a {what} before continuing."));
    }
    if !path.is_absolute() {
        return Err(format!(
            "{what} must be an absolute path ({}). Relative paths and PATH lookup are not used.",
            path.display()
        ));
    }
    let meta =
        fs::metadata(path).map_err(|_| format!("{what} does not exist: {}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{what} is not a file: {}", path.display()));
    }
    Ok(path.to_path_buf())
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn normalize_dir(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return fs::canonicalize(path)
            .map_err(|err| format!("could not resolve {}: {err}", path.display()));
    }
    Ok(lexical_normalize(path))
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|err| format!("could not hash {}: {err}", path.display()))?;
    let digest = Sha256::digest(bytes);
    Ok(format!("{digest:x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    fn executable(dir: &Path) -> PathBuf {
        let path = dir.join("kovanica-node");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, "#!/bin/sh\nexit 0").unwrap();
        let mut permissions = file.metadata().unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        path
    }

    fn write_config(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("node.toml");
        fs::write(&path, body).unwrap();
        path
    }

    fn ok_body(data_dir: &Path) -> String {
        format!(
            "chain_id = 1337\ndata_dir = \"{}\"\nrpc_addr = \"127.0.0.1\"\nrpc_port = 8545\nrun_validator = false\n\n[mining]\nenabled = false\n",
            data_dir.display()
        )
    }

    #[test]
    fn accepts_loopback_non_validator_config() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("chaindata");
        fs::create_dir(&data).unwrap();
        let binary = executable(tmp.path());
        let config = write_config(tmp.path(), &ok_body(&data));
        let checked = check_setup(&binary, &config, &data, &[]).unwrap();
        assert_eq!(checked.chain_id, 1337);
        assert!(checked.chain_id_from_file);
        assert!(!checked.run_validator);
        assert!(!checked.mining_enabled);
        assert!(checked.binary_sha256.len() == 64);
        assert_eq!(checked.rpc_http, "http://127.0.0.1:8545");
    }

    #[test]
    fn rejects_mainnet_public_rpc_mining_and_validator() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("chaindata");
        let binary = executable(tmp.path());
        let mainnet = write_config(
            tmp.path(),
            &format!(
                "chain_id = 1\ndata_dir = \"{}\"\nrpc_addr = \"127.0.0.1\"\nrun_validator = false\n",
                data.display()
            ),
        );
        let err = check_setup(&binary, &mainnet, &data, &[]).unwrap_err();
        assert!(err.contains("mainnet"), "{err}");

        let public_rpc = write_config(
            tmp.path(),
            &format!(
                "chain_id = 4\ndata_dir = \"{}\"\nrpc_addr = \"0.0.0.0\"\nrun_validator = false\n",
                data.display()
            ),
        );
        let err = check_setup(&binary, &public_rpc, &data, &[]).unwrap_err();
        assert!(err.contains("loopback"), "{err}");

        let mining = write_config(
            tmp.path(),
            &format!(
                "chain_id = 5\ndata_dir = \"{}\"\nrpc_addr = \"127.0.0.1\"\nrun_validator = false\n\n[mining]\nenabled = true\n",
                data.display()
            ),
        );
        let err = check_setup(&binary, &mining, &data, &[]).unwrap_err();
        assert!(err.contains("mining"), "{err}");

        let validator = write_config(
            tmp.path(),
            &format!(
                "chain_id = 4\ndata_dir = \"{}\"\nrpc_addr = \"127.0.0.1\"\nrun_validator = true\n",
                data.display()
            ),
        );
        let err = check_setup(&binary, &validator, &data, &[]).unwrap_err();
        assert!(err.contains("run_validator"), "{err}");
    }

    #[test]
    fn rejects_omitted_validator_flag_relative_data_dir_and_env_overrides() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("chaindata");
        let binary = executable(tmp.path());
        let omitted = write_config(
            tmp.path(),
            &format!(
                "chain_id = 1337\ndata_dir = \"{}\"\nrpc_addr = \"127.0.0.1\"\n",
                data.display()
            ),
        );
        let err = check_setup(&binary, &omitted, &data, &[]).unwrap_err();
        assert!(err.contains("defaults it to true"), "{err}");

        let relative = write_config(
            tmp.path(),
            "chain_id = 1337\ndata_dir = \"data\"\nrpc_addr = \"127.0.0.1\"\nrun_validator = false\n",
        );
        let err = check_setup(&binary, &relative, &data, &[]).unwrap_err();
        assert!(err.contains("relative"), "{err}");

        let config = write_config(tmp.path(), &ok_body(&data));
        let err =
            check_setup(&binary, &config, &data, &["KOVANICA_CHAIN_ID".to_string()]).unwrap_err();
        assert!(err.contains("KOVANICA_CHAIN_ID"), "{err}");
        assert!(!err.contains('='));
    }

    #[test]
    fn rejects_mismatched_data_dir_without_rewriting_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let configured = tmp.path().join("configured");
        let selected = tmp.path().join("other");
        let binary = executable(tmp.path());
        let body = ok_body(&configured);
        let config = write_config(tmp.path(), &body);
        let before = fs::read(&config).unwrap();
        let err = check_setup(&binary, &config, &selected, &[]).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
        assert_eq!(fs::read(&config).unwrap(), before);
    }

    #[test]
    fn rejects_relative_binary_and_unknown_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("chaindata");
        let binary = executable(tmp.path());
        let config = write_config(tmp.path(), &ok_body(&data));
        let err = check_setup(Path::new("kovanica-node"), &config, &data, &[]).unwrap_err();
        assert!(err.contains("absolute"), "{err}");

        let missing = tmp.path().join("missing-binary");
        let err = check_setup(&missing, &config, &data, &[]).unwrap_err();
        assert!(err.contains("does not exist"), "{err}");

        let unknown = write_config(
            tmp.path(),
            &format!(
                "chain_id = 99\ndata_dir = \"{}\"\nrun_validator = false\nrpc_addr = \"127.0.0.1\"\n",
                data.display()
            ),
        );
        let err = check_setup(&binary, &unknown, &data, &[]).unwrap_err();
        assert!(err.contains("not a supported"), "{err}");
        assert!(!err.contains("testnet"), "{err}");
    }

    #[test]
    fn env_name_filter_ignores_values() {
        let names = kovanica_env_names(["PATH", "KOVANICA_RPC_ADDR", "HOME", "KOVANICA_RPC_ADDR"]);
        assert_eq!(names, vec!["KOVANICA_RPC_ADDR".to_string()]);
    }

    #[test]
    fn paths_with_spaces_are_accepted_when_they_match() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("dir with spaces");
        fs::create_dir(&dir).unwrap();
        let data = dir.join("chain data");
        fs::create_dir(&data).unwrap();
        let binary = executable(&dir);
        let config = dir.join("my config.toml");
        fs::write(&config, ok_body(&data)).unwrap();
        let checked = check_setup(&binary, &config, &data, &[]).unwrap();
        assert!(checked.config_path.contains("my config.toml"));
        assert!(checked.data_dir.contains("chain data"));
    }
}
