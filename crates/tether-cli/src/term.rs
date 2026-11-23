//! Making text from other devices safe to print in a terminal.
//!
//! Clipboard text and device names are chosen by other devices. A terminal
//! acts on control characters: ESC starts sequences that can write the
//! clipboard (OSC 52), retitle the window, move the cursor and overwrite
//! earlier lines; C1 characters do the same on some terminals; bidi controls
//! reorder what is shown. Everything from the daemon goes through [`safe`]
//! before it is printed (SECURITY.md, "`tether` CLI").

use tether_core::DeviceId;

/// Bidi overrides, embeddings, isolates and marks.
fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}')
}

/// `text` with everything a terminal would act on made visible and inert.
/// `multiline` keeps newlines and tabs (for full output); otherwise a newline
/// shows as `⏎` and a tab as a space.
pub fn safe(text: &str, multiline: bool) -> String {
    text.chars()
        .map(|c| match c {
            '\n' if multiline => '\n',
            '\t' if multiline => '\t',
            '\n' => '⏎',
            '\t' => ' ',
            // C0 controls, ESC included, as Unicode control pictures (␛ for ESC).
            '\u{0}'..='\u{1f}' => char::from_u32(0x2400 + c as u32).unwrap_or('\u{FFFD}'),
            '\u{7f}' => '␡',
            '\u{80}'..='\u{9f}' => '\u{FFFD}',
            c if is_bidi_control(c) => '\u{FFFD}',
            c => c,
        })
        .collect()
}

/// A one-line, safe preview of at most `max` characters.
pub fn preview(text: &str, max: usize) -> String {
    let line = safe(text.trim(), false);
    if line.chars().count() <= max {
        return line;
    }
    let mut cut: String = line.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// "just now", "5 s ago", "3 min ago", "2 h ago", "4 d ago".
pub fn ago(wall_ms: u64, now_ms: u64) -> String {
    let secs = now_ms.saturating_sub(wall_ms) / 1000;
    match secs {
        0..=1 => "just now".into(),
        2..=59 => format!("{secs} s ago"),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// The first 12 hex digits of a device id: enough to tell devices apart and
/// to type into `tether revoke`.
pub fn short(id: &DeviceId) -> String {
    id.to_hex()[..12].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inert(s: &str) -> bool {
        !s.chars().any(|c| (c.is_control() && c != '\n' && c != '\t') || is_bidi_control(c))
    }

    #[test]
    fn escape_sequences_are_made_visible_and_inert() {
        let attacks = [
            "\u{1b}]52;c;Y3VybCBldmlsLnNoIHwgc2g=\u{7}", // OSC 52: write the clipboard
            "\u{1b}]0;totally a title\u{7}",             // retitle the window
            "\u{1b}[2J\u{1b}[H fake prompt $ ",          // clear the screen, home the cursor
            "line\r overwritten",                        // carriage return
            "\u{9b}31m C1 CSI",                          // 8-bit CSI
            "\u{9d}52;c;x\u{9c}",                        // 8-bit OSC and ST
            "invoice\u{202E}fdp.exe",                    // right-to-left override
            "a\u{2066}b\u{2069}c\u{200F}",               // isolates and marks
            "bell\u{7} null\u{0} del\u{7f}",
        ];
        for a in attacks {
            for multiline in [false, true] {
                let out = safe(a, multiline);
                assert!(inert(&out), "{a:?} -> {out:?}");
            }
        }
        assert_eq!(safe("\u{1b}[31m", false), "␛[31m");
        assert_eq!(safe("a\u{202E}b", false), "a\u{FFFD}b");
    }

    #[test]
    fn newlines_are_kept_only_in_full_output() {
        assert_eq!(safe("one\ntwo\tx", true), "one\ntwo\tx");
        assert_eq!(safe("one\ntwo\tx", false), "one⏎two x");
    }

    #[test]
    fn previews_are_one_safe_line_of_bounded_length() {
        assert_eq!(preview("  short  ", 10), "short");
        let p = preview(&"é".repeat(50), 10);
        assert_eq!((p.chars().count(), p.ends_with('…')), (10, true));
        assert!(!preview("a\nb\u{1b}c", 80).contains(['\n', '\u{1b}']));
    }

    #[test]
    fn times_read_naturally() {
        let now = 10_000_000;
        assert_eq!(ago(now, now), "just now");
        assert_eq!(ago(now - 5_000, now), "5 s ago");
        assert_eq!(ago(now - 180_000, now), "3 min ago");
        assert_eq!(ago(now - 7_200_000, now), "2 h ago");
        assert_eq!(ago(now + 50_000, now), "just now", "a clock ahead of ours is not the future");
    }
}
