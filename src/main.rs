mod feedback;
mod solver;
mod wordlist;

use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use clap::Parser;

use solver::Guess;

/// Suggest next Wordle guesses ranked by expected information (bits).
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Past guesses as "WORD PATTERN", e.g. "crane XGXYX"
    /// (G=green, Y=yellow, X=gray; case-insensitive)
    guesses: Vec<String>,

    /// Path to a wordlist file, one word per line (default: bundled Wordle guess list)
    #[arg(long, value_name = "PATH")]
    wordlist: Option<PathBuf>,

    /// Number of suggestions to print
    #[arg(short = 'n', long = "top", default_value_t = 10, value_name = "N")]
    top: usize,

    /// Only suggest words that are still possible answers
    #[arg(long)]
    candidates_only: bool,
}

fn parse_guess(arg: &str, word_len: usize) -> Result<Guess> {
    let mut parts = arg.split_whitespace();
    let (Some(word), Some(pattern), None) = (parts.next(), parts.next(), parts.next()) else {
        bail!("expected \"WORD PATTERN\", e.g. \"crane XGXYX\"");
    };
    ensure!(
        word.chars().all(|c| c.is_ascii_alphabetic()),
        "word \"{word}\" contains non-alphabetic characters"
    );
    ensure!(
        word.len() == word_len,
        "word \"{word}\" has length {}, expected {word_len}",
        word.len()
    );
    Ok(Guess {
        word: word.to_ascii_lowercase().into_bytes(),
        pattern: feedback::parse_pattern(pattern, word_len)?,
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (words, word_len) = wordlist::load(cli.wordlist.as_deref())?;

    let mut guesses = Vec::new();
    for (i, arg) in cli.guesses.iter().enumerate() {
        let guess = parse_guess(arg, word_len)
            .with_context(|| format!("guess {} (\"{arg}\") is invalid", i + 1))?;
        guesses.push(guess);
    }

    if let Some(solved) = guesses
        .iter()
        .find(|g| g.pattern.iter().all(|&c| c == feedback::GREEN))
    {
        println!("Solved: {}", String::from_utf8_lossy(&solved.word));
        return Ok(());
    }

    for g in &guesses {
        if words
            .binary_search_by(|w| w.as_slice().cmp(&g.word))
            .is_err()
        {
            eprintln!(
                "warning: guessed word \"{}\" is not in the wordlist",
                String::from_utf8_lossy(&g.word)
            );
        }
    }

    let candidates = solver::filter_candidates(&words, &guesses);
    match candidates.len() {
        0 => bail!(
            "no candidates match these guesses — check the patterns for typos or a wordlist mismatch"
        ),
        1 => {
            println!(
                "Only one candidate remains: {}",
                String::from_utf8_lossy(candidates[0])
            );
            return Ok(());
        }
        _ => {}
    }

    println!(
        "Candidates remaining: {} of {}",
        candidates.len(),
        words.len()
    );

    let full_pool: Vec<&[u8]>;
    let pool: &[&[u8]] = if cli.candidates_only {
        &candidates
    } else {
        full_pool = words.iter().map(Vec::as_slice).collect();
        &full_pool
    };

    let suggestions = solver::rank_guesses(pool, &candidates, cli.top, word_len);

    println!();
    println!("Top guesses by expected information:");
    let width = suggestions.iter().map(|s| s.word.len()).max().unwrap_or(0);
    let mut any_candidate = false;
    for (i, s) in suggestions.iter().enumerate() {
        let marker = if s.is_candidate {
            any_candidate = true;
            "   *"
        } else {
            ""
        };
        println!(
            "{:>4}. {:<width$}   {:>5.2} bits{marker}",
            i + 1,
            s.word,
            s.bits
        );
    }
    if any_candidate && !cli.candidates_only {
        println!();
        println!("* = still a possible answer");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use feedback::{GRAY, GREEN, YELLOW};

    #[test]
    fn parse_guess_happy_path() {
        let g = parse_guess("GREEN GXYYX", 5).unwrap();
        assert_eq!(g.word, b"green");
        assert_eq!(g.pattern, [GREEN, GRAY, YELLOW, YELLOW, GRAY]);
    }

    #[test]
    fn parse_guess_requires_two_tokens() {
        assert!(parse_guess("green", 5).is_err());
        assert!(parse_guess("green GXYYX extra", 5).is_err());
    }

    #[test]
    fn parse_guess_rejects_wrong_word_length() {
        let err = parse_guess("greens GXYYXG", 5).unwrap_err().to_string();
        assert!(err.contains("has length 6, expected 5"), "{err}");
    }

    #[test]
    fn parse_guess_rejects_wrong_pattern_length() {
        let err = parse_guess("green GXYY", 5).unwrap_err().to_string();
        assert!(err.contains("has length 4, expected 5"), "{err}");
    }

    #[test]
    fn parse_guess_rejects_non_alphabetic_word() {
        let err = parse_guess("gr33n GXYYX", 5).unwrap_err().to_string();
        assert!(err.contains("non-alphabetic"), "{err}");
    }
}
