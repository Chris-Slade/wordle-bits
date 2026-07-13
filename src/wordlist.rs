use std::path::Path;

use anyhow::{Context, Result, bail, ensure};

use crate::feedback::MAX_WORD_LEN;

/// The standard Wordle valid-guess list, bundled so the binary works out of the box.
pub const DEFAULT_WORDLIST: &str = include_str!("../words/valid-guesses.txt");

/// Load a wordlist from `path`, or the bundled default when `path` is `None`.
///
/// Words are lowercased, deduplicated, and sorted. The list must be non-empty,
/// ASCII-alphabetic, and of uniform word length; that length is returned and
/// drives validation of the guesses.
pub fn load(path: Option<&Path>) -> Result<(Vec<Vec<u8>>, usize)> {
    match path {
        Some(p) => {
            let contents = std::fs::read_to_string(p)
                .with_context(|| format!("failed to read wordlist {}", p.display()))?;
            parse(&contents)
        }
        None => parse(DEFAULT_WORDLIST),
    }
}

fn parse(contents: &str) -> Result<(Vec<Vec<u8>>, usize)> {
    let mut words: Vec<Vec<u8>> = Vec::new();
    let mut word_len = None;
    for (idx, line) in contents.lines().enumerate() {
        let word = line.trim();
        if word.is_empty() {
            continue;
        }
        ensure!(
            word.chars().all(|c| c.is_ascii_alphabetic()),
            "wordlist line {}: \"{word}\" contains non-ASCII-alphabetic characters",
            idx + 1
        );
        ensure!(
            word.len() <= MAX_WORD_LEN,
            "wordlist line {}: words longer than {MAX_WORD_LEN} letters are not supported",
            idx + 1
        );
        match word_len {
            None => word_len = Some(word.len()),
            Some(len) => ensure!(
                word.len() == len,
                "wordlist mixes word lengths: line {} has \"{word}\" (length {}) after length-{len} words",
                idx + 1,
                word.len()
            ),
        }
        words.push(word.to_ascii_lowercase().into_bytes());
    }
    let Some(word_len) = word_len else {
        bail!("wordlist is empty");
    };
    words.sort();
    words.dedup();
    Ok((words, word_len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_dedups_and_sorts() {
        let (words, len) = parse("CRANE\n  slate \ncrane\n\ntrace\n").unwrap();
        assert_eq!(len, 5);
        assert_eq!(words, [b"crane", b"slate", b"trace"]);
    }

    #[test]
    fn word_length_comes_from_the_list() {
        let (words, len) = parse("cranes\nslated\n").unwrap();
        assert_eq!(len, 6);
        assert_eq!(words.len(), 2);
    }

    #[test]
    fn rejects_mixed_lengths() {
        let err = parse("crane\nslated\n").unwrap_err().to_string();
        assert!(err.contains("mixes word lengths"), "{err}");
    }

    #[test]
    fn rejects_non_alphabetic() {
        let err = parse("cr4ne\n").unwrap_err().to_string();
        assert!(err.contains("non-ASCII-alphabetic"), "{err}");
    }

    #[test]
    fn rejects_empty_list() {
        let err = parse("\n  \n").unwrap_err().to_string();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn default_wordlist_loads() {
        let (words, len) = load(None).unwrap();
        assert_eq!(len, 5);
        assert!(words.len() > 10_000);
    }
}
