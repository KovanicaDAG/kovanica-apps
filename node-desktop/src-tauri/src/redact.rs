//! Log redaction. Lines that look like secrets are dropped whole.
//! The supervisor never writes the config file into the log buffer.

const SENSITIVE: &[&str] = &[
    "passphrase",
    "mnemonic",
    "seed phrase",
    "private key",
    "private_key",
    "authorization",
    "bearer ",
    "bearer:",
    "write_token",
    "keystore",
    "secret",
];

const MAX_LINE: usize = 500;

/// Returns a display-safe line, or `None` when the line must not be shown.
pub fn redact_log_line(line: &str) -> Option<String> {
    let trimmed = line.trim_end_matches(['\r', '\n']);
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if SENSITIVE.iter().any(|needle| lower.contains(needle)) {
        return Some("[redacted]".to_string());
    }
    if trimmed.chars().count() <= MAX_LINE {
        return Some(trimmed.to_string());
    }
    let truncated: String = trimmed.chars().take(MAX_LINE).collect();
    Some(format!("{truncated}…"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secret_lines_and_keeps_ordinary_logs() {
        assert_eq!(
            redact_log_line("passphrase=hunter2").as_deref(),
            Some("[redacted]")
        );
        assert_eq!(
            redact_log_line("Authorization: Bearer sekrit").as_deref(),
            Some("[redacted]")
        );
        assert_eq!(
            redact_log_line("listening on 127.0.0.1:8545").as_deref(),
            Some("listening on 127.0.0.1:8545")
        );
        assert_eq!(redact_log_line("\n").as_deref(), None);
    }
}
