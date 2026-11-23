//! Where the window may go, and which links the app may open. SECURITY.md
//! rule 10: the UI loads nothing remote; rule 3: received links open only on
//! an explicit local action, and only in the user's own browser or mail app.

use tauri::Url;

/// Longest link `open_link` accepts.
const MAX_LINK_LEN: usize = 2048;

/// Is `url` the app's own bundled page? Anything else is refused as a
/// navigation target, so a link in clipboard text can never load a remote
/// page into the app's window (where it would sit next to the app's commands).
pub fn is_app_url(url: &Url, dev: bool) -> bool {
    match (url.scheme(), url.host_str()) {
        // macOS and Linux serve the bundle at tauri://localhost, Windows at http(s)://tauri.localhost.
        ("tauri", Some("localhost")) => true,
        ("http" | "https", Some("tauri.localhost")) => url.port().is_none(),
        // `tauri dev` serves the page from Vite (vite.config.ts pins the port).
        ("http", Some("localhost")) => dev && url.port() == Some(1420),
        _ => false,
    }
}

/// A link from the page that may be handed to the OS: http, https or mailto,
/// no embedded credentials, no control characters, bounded length. The
/// returned form is the normalised one, which is what gets opened.
pub fn external_link(raw: &str) -> Result<Url, String> {
    if raw.len() > MAX_LINK_LEN {
        return Err("that link is too long to open".into());
    }
    if raw.chars().any(char::is_control) {
        return Err("that link contains control characters".into());
    }
    let url = Url::parse(raw.trim()).map_err(|_| "that is not a link".to_string())?;
    match url.scheme() {
        "http" | "https" => {
            if !matches!(url.host_str(), Some(host) if !host.is_empty()) {
                return Err("that link has no host".into());
            }
            if !url.username().is_empty() || url.password().is_some() {
                return Err("links with a user name or password in them are not opened".into());
            }
        }
        "mailto" => {}
        _ => return Err("only web and email links can be opened".into()),
    }
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn only_the_bundled_page_is_a_navigation_target() {
        for ok in [
            "tauri://localhost/",
            "tauri://localhost/index.html#clipboard",
            "http://tauri.localhost/",
            "https://tauri.localhost/x",
        ] {
            assert!(is_app_url(&url(ok), false), "{ok} refused");
        }
        assert!(is_app_url(&url("http://localhost:1420/#devices"), true));
        for bad in [
            "http://localhost:1420/", // the dev server, outside dev builds
            "https://example.com/",
            "http://tauri.localhost.evil.example/",
            "http://tauri.localhost:8080/",
            "http://evil.example/?h=tauri.localhost",
            "file:///C:/Windows/System32/",
            "about:blank",
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
        ] {
            assert!(!is_app_url(&url(bad), false), "{bad} allowed");
        }
        assert!(!is_app_url(&url("http://localhost:3000/"), true), "only Vite's port in dev");
    }

    #[test]
    fn only_web_and_mail_links_are_opened() {
        assert_eq!(
            external_link("https://github.com/tokio-rs/tokio/pull/6812").unwrap().as_str(),
            "https://github.com/tokio-rs/tokio/pull/6812"
        );
        assert!(external_link("http://192.168.1.20:8080/x?y=1").is_ok());
        assert!(external_link("mailto:someone@example.com").is_ok());
        for bad in [
            "file:///C:/Windows/System32/calc.exe",
            r"\\attacker.example\share\x.lnk",
            "C:\\Windows\\System32\\calc.exe",
            "javascript:alert(1)",
            "ms-msdt:/id PCWDiagnostic",
            "search-ms:query=x&crumb=location:\\\\evil\\share",
            "vbscript:msgbox",
            "https://user:pass@example.com/",
            "https://example.com/\u{1b}]52;c;aGk=\u{7}",
            "https://",
            "not a link",
        ] {
            assert!(external_link(bad).is_err(), "{bad:?} would be opened");
        }
        assert!(external_link(&format!("https://example.com/{}", "a".repeat(MAX_LINK_LEN))).is_err());
    }
}
