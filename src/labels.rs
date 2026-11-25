//! Label detection: which short tokens of a turn look like names (spec 6.1).

use std::collections::HashSet;

/// Common acronyms that are never treated as labels.
const BUILTIN_STOPLIST: &[&str] = &[
    "OK", "API", "URL", "URI", "ID", "TODO", "DKIM", "SPF", "DMARC", "JSON", "YAML", "SQL", "HTTP",
    "HTTPS", "CLI", "UI", "PR", "CI", "CD", "DB", "OS", "AI", "LLM", "MCP", "SSH", "VPN", "DNS",
    "TLS", "SSL", "IP", "CPU", "GPU", "RAM", "PDF", "CSV", "HTML", "CSS", "NOTE", "WARN", "ERROR",
    "INFO", "DEBUG", "README", "MIT", "GPL",
    // Git and shell words; `ORIG_HEAD` and `FETCH_HEAD` split at `_` into `ORIG`/`FETCH` + `HEAD`.
    "HEAD", "ORIG", "FETCH", "MERGE", "EOF", "PATH", "ENV", "SHA", "PID", "TTY", "STDIN", "STDOUT",
    "STDERR",
];

/// Built-in stoplist plus user additions, compared case-insensitively.
pub struct Stoplist {
    keys: HashSet<String>,
}

impl Stoplist {
    pub fn new(user: &[String]) -> Self {
        let keys = BUILTIN_STOPLIST
            .iter()
            .map(|s| s.to_lowercase())
            .chain(user.iter().map(|s| s.to_lowercase()))
            .collect();
        Stoplist { keys }
    }

    pub fn contains(&self, label: &str) -> bool {
        self.keys.contains(&label.to_lowercase())
    }
}

/// Splits `text` into maximal alphanumeric runs, with their byte ranges.
fn tokens(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text.char_indices() {
        match (c.is_alphanumeric(), start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                out.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, text.len()));
    }
    out
}

/// Uppercase word of 2-8 letters, e.g. `OLD`.
fn is_upper_word(t: &str) -> bool {
    let n = t.chars().count();
    (2..=8).contains(&n) && t.chars().all(|c| c.is_alphabetic() && c.is_uppercase())
}

/// Letter+digit mix of 2-4 characters, e.g. `W6`, `8a`.
fn is_letter_digit_mix(t: &str) -> bool {
    let n = t.chars().count();
    (2..=4).contains(&n) && t.chars().any(char::is_alphabetic) && t.chars().any(|c| c.is_numeric())
}

/// Capitalised word of at least two letters, e.g. `Decision`.
fn is_capitalised_word(t: &str) -> bool {
    let mut cs = t.chars();
    match cs.next() {
        Some(first) if first.is_alphabetic() && first.is_uppercase() => {
            let rest: Vec<char> = cs.collect();
            !rest.is_empty() && rest.iter().all(|c| c.is_alphabetic() && c.is_lowercase())
        }
        _ => false,
    }
}

/// One to three ASCII digits, e.g. the `1` of `Decision 1`.
fn is_small_number(t: &str) -> bool {
    (1..=3).contains(&t.len()) && t.chars().all(|c| c.is_ascii_digit())
}

/// Label-shaped tokens of `text`, deduplicated, in first-seen order.
pub fn candidates(text: &str, stop: &Stoplist) -> Vec<String> {
    let toks = tokens(text);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut push = |label: &str| {
        if !stop.contains(label) && seen.insert(label.to_lowercase()) {
            out.push(label.to_string());
        }
    };
    for (i, &(s, e)) in toks.iter().enumerate() {
        let t = &text[s..e];
        if is_upper_word(t) || is_letter_digit_mix(t) {
            push(t);
        } else if is_capitalised_word(t) {
            if let Some(&(ns, ne)) = toks.get(i + 1) {
                if &text[e..ns] == " " && is_small_number(&text[ns..ne]) {
                    push(&text[s..ne]);
                }
            }
        }
    }
    out
}

/// Whether `hay` contains `needle` delimited by non-alphanumerics.
fn contains_word(hay: &str, needle: &str) -> bool {
    hay.match_indices(needle).any(|(i, m)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + m.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Entries of `known` that occur in `text` as whole words (case-insensitive);
/// returns the stored spelling.
pub fn mentions(text: &str, known: &[String]) -> Vec<String> {
    let lowered = text.to_lowercase();
    known
        .iter()
        .filter(|label| {
            let key = label.to_lowercase();
            !key.is_empty() && contains_word(&lowered, &key)
        })
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cands(text: &str) -> Vec<String> {
        candidates(text, &Stoplist::new(&[]))
    }

    #[test]
    fn finds_old_new_w6_d4_8a() {
        let t = "Move from OLD to NEW in window W6, then run D4 and ask session 8a.";
        assert_eq!(cands(t), ["OLD", "NEW", "W6", "D4", "8a"]);
    }

    #[test]
    fn finds_decision_and_karar_n() {
        assert_eq!(
            cands("See Decision 1 and Karar 12; Decision 1 again."),
            ["Decision 1", "Karar 12"]
        );
    }

    #[test]
    fn skips_commit_hash_61797a6() {
        assert!(cands("fixed in 61797a6 yesterday").is_empty());
    }

    #[test]
    fn skips_builtin_stoplist_ok_api() {
        assert!(cands("OK, the API returns JSON").is_empty());
    }

    #[test]
    fn skips_git_and_shell_words() {
        let t = "HEAD 1992ad0, ORIG_HEAD, FETCH_HEAD, MERGE_HEAD, EOF, PATH, ENV, SHA, PID, TTY, STDIN, STDOUT, STDERR";
        assert!(cands(t).is_empty(), "{:?}", cands(t));
    }

    #[test]
    fn user_stoplist_is_case_insensitive() {
        let stop = Stoplist::new(&["old".to_string()]);
        assert_eq!(candidates("OLD and NEW", &stop), ["NEW"]);
    }

    #[test]
    fn plain_prose_has_no_candidates() {
        assert!(cands("Bu üçü tamamlanınca başlatacağım.").is_empty());
    }

    #[test]
    fn mentions_are_whole_word_case_insensitive() {
        let known = vec!["W6".to_string(), "Karar 1".to_string(), "OLD".to_string()];
        assert_eq!(
            mentions("w6 is set; karar 1 holds; bold W66", &known),
            ["W6", "Karar 1"]
        );
    }

    #[test]
    fn mentions_handle_unicode_lowercase() {
        let known = vec!["ÇAY".to_string()];
        assert_eq!(mentions("bu çay için", &known), ["ÇAY"]);
    }
}
