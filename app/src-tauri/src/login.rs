//! Opening Tether at sign-in: a per-user login entry (the Windows `Run` key,
//! a macOS launch agent, an XDG autostart file on Linux), off until the user
//! turns it on in the app.
//!
//! The entry is a command line the OS runs at every sign-in, so the path in
//! it is quoted. Unquoted, a path with a space (`C:\Users\Ann Lee\...`) would
//! make Windows try `C:\Users\Ann.exe` first: anyone who can create that file
//! gets code run at sign-in (CWE-428). auto-launch writes paths as given, so
//! it is given a quoted one.

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

pub const DEFAULT_PROFILE: &str = "default";

/// What the login entry runs: this app, hidden, for this profile.
pub fn login_args(profile: &str) -> Vec<String> {
    let mut args = vec!["--hidden".to_string()];
    if profile != DEFAULT_PROFILE {
        args.extend(["--profile".to_string(), profile.to_string()]);
    }
    args
}

/// The login entry's name: one per profile, so two never overwrite each other.
pub fn login_entry_name(profile: &str) -> String {
    if profile == DEFAULT_PROFILE {
        "Tether".to_string()
    } else {
        format!("Tether ({profile})")
    }
}

/// `exe` as the first word of a Windows command line.
pub fn windows_command_path(exe: &str) -> Result<String, String> {
    // A Windows path cannot contain a double quote, but refuse one anyway.
    if exe.contains('"') {
        return Err("the app's path contains a quote".into());
    }
    Ok(format!("\"{exe}\""))
}

/// `exe` as the first word of a desktop entry's `Exec=` line. Characters that
/// need escaping there are refused rather than escaped (fail closed).
pub fn desktop_entry_path(exe: &str) -> Result<String, String> {
    if exe.chars().any(|c| matches!(c, '"' | '`' | '$' | '\\' | '%' | '\n' | '\r')) {
        return Err("the app's folder has characters a login entry cannot hold; move it to open it at sign-in".into());
    }
    Ok(format!("\"{exe}\""))
}

/// The login entry for this executable and profile.
pub fn entry(profile: &str) -> Result<AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| format!("finding the app's own executable: {e}"))?;
    let exe = exe.to_str().ok_or("the app's path is not valid Unicode")?;
    let path = if cfg!(windows) {
        windows_command_path(exe)?
    } else if cfg!(target_os = "linux") {
        desktop_entry_path(exe)?
    } else {
        // macOS launch agents take an argument array: no quoting.
        exe.to_string()
    };
    AutoLaunchBuilder::new()
        .set_app_name(&login_entry_name(profile))
        .set_app_path(&path)
        .set_args(&login_args(profile))
        .build()
        .map_err(|e| format!("preparing the login entry: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_with_spaces_are_quoted() {
        let exe = r"C:\Users\Ann Lee\AppData\Local\Tether\tether-app.exe";
        let line = format!("{} {}", windows_command_path(exe).unwrap(), login_args("default").join(" "));
        assert_eq!(line, r#""C:\Users\Ann Lee\AppData\Local\Tether\tether-app.exe" --hidden"#);
        assert!(windows_command_path(r#"C:\a"b.exe"#).is_err());
    }

    #[test]
    fn desktop_entries_refuse_what_they_cannot_hold() {
        assert_eq!(desktop_entry_path("/home/ann lee/tether-app").unwrap(), "\"/home/ann lee/tether-app\"");
        for bad in ["/home/$USER/x", "/opt/`id`/x", "/opt/100%/x", "/opt/a\"b/x", "/opt/a\\b/x", "/opt/a\nExec=x"] {
            assert!(desktop_entry_path(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn profiles_get_separate_entries() {
        assert_ne!(login_entry_name("default"), login_entry_name("left"));
        assert_eq!(login_args("left"), ["--hidden", "--profile", "left"]);
    }
}
