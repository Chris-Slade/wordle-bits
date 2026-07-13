mod feedback;
mod solver;
mod wordlist;

use std::io::{BufRead, IsTerminal, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, ValueEnum};

use solver::Guess;

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum ColorMode {
    Always,
    Auto,
    Never,
}

/// Suggest next Wordle guesses ranked by expected information (bits).
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Past guesses as WORD PATTERN pairs, quoted or not:
    /// `crane XGXYX` or `"crane XGXYX"` (G=green, Y=yellow, X=gray; case-insensitive)
    guesses: Vec<String>,

    /// Path to a wordlist file, one word per line (default: bundled Wordle guess list)
    #[arg(long, value_name = "PATH")]
    wordlist: Option<PathBuf>,

    /// Optional answer list: candidates are drawn from it instead of the
    /// primary wordlist, while suggestions still come from the primary list
    #[arg(long, value_name = "PATH")]
    answers: Option<PathBuf>,

    /// Number of suggestions to print
    #[arg(short = 'n', long = "top", default_value_t = 10, value_name = "N")]
    top: usize,

    /// Only suggest words that are still possible answers
    #[arg(long)]
    candidates_only: bool,

    /// Evaluate how a specific guess ranks against the suggestion pool
    /// (may be given multiple times)
    #[arg(short = 'q', long, value_name = "WORD")]
    query: Vec<String>,

    /// Read further guesses from stdin, showing the game state each round
    #[arg(short = 'i', long)]
    interactive: bool,

    /// When to use ANSI colors to display guesses
    #[arg(long, value_enum, default_value_t = ColorMode::Auto)]
    color: ColorMode,
}

/// Pair up guess tokens from the command line. Each positional argument may be
/// a bare WORD or PATTERN token, or a quoted "WORD PATTERN" pair.
fn parse_guesses(args: &[String], word_len: usize) -> Result<Vec<Guess>> {
    let tokens: Vec<&str> = args.iter().flat_map(|a| a.split_whitespace()).collect();
    ensure!(
        tokens.len().is_multiple_of(2),
        "expected WORD PATTERN pairs, but got an odd number of tokens ({})",
        tokens.len()
    );
    tokens
        .chunks(2)
        .enumerate()
        .map(|(i, pair)| {
            parse_guess(pair[0], pair[1], word_len).with_context(|| {
                format!("guess {} (\"{} {}\") is invalid", i + 1, pair[0], pair[1])
            })
        })
        .collect()
}

fn parse_word(word: &str, word_len: usize) -> Result<Vec<u8>> {
    ensure!(
        word.chars().all(|c| c.is_ascii_alphabetic()),
        "word \"{word}\" contains non-alphabetic characters"
    );
    ensure!(
        word.len() == word_len,
        "word \"{word}\" has length {}, expected {word_len}",
        word.len()
    );
    Ok(word.to_ascii_lowercase().into_bytes())
}

fn parse_guess(word: &str, pattern: &str, word_len: usize) -> Result<Guess> {
    Ok(Guess {
        word: parse_word(word, word_len)?,
        pattern: feedback::parse_pattern(pattern, word_len)?,
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let (words, word_len) = wordlist::load(cli.wordlist.as_deref())?;
    let answers = match &cli.answers {
        Some(path) => {
            let (answers, answers_len) = wordlist::load(Some(path))?;
            ensure!(
                answers_len == word_len,
                "answer list word length ({answers_len}) does not match wordlist word length ({word_len})"
            );
            Some(answers)
        }
        None => None,
    };
    let answer_pool: &[Vec<u8>] = answers.as_deref().unwrap_or(&words);
    let color = match cli.color {
        ColorMode::Always => true,
        ColorMode::Never => false,
        ColorMode::Auto => std::io::stdout().is_terminal(),
    };

    let mut guesses = parse_guesses(&cli.guesses, word_len)?;
    warn_unknown_words(&words, guesses.iter().map(|g| g.word.as_slice()));
    let queries: Vec<Vec<u8>> = cli
        .query
        .iter()
        .map(|w| parse_word(w, word_len).with_context(|| format!("query \"{w}\" is invalid")))
        .collect::<Result<_>>()?;
    warn_unknown_words(&words, queries.iter().map(Vec::as_slice));

    if !cli.interactive {
        report(&words, answer_pool, &guesses, &queries, &cli, word_len)?;
        return Ok(());
    }

    println!("Enter guesses as WORD PATTERN (G=green, Y=yellow, X=gray); Ctrl-D to quit.");
    let stdin = std::io::stdin();
    loop {
        show_state(&guesses, color);
        if report(&words, answer_pool, &guesses, &queries, &cli, word_len)? {
            return Ok(());
        }

        print!("\n> ");
        std::io::stdout().flush()?;
        let mut line = String::new();
        if stdin.lock().read_line(&mut line)? == 0 {
            println!();
            return Ok(());
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match parse_guesses(&[line.to_string()], word_len) {
            Ok(new_guesses) => {
                warn_unknown_words(&words, new_guesses.iter().map(|g| g.word.as_slice()));
                guesses.extend(new_guesses);
            }
            Err(e) => eprintln!("Error: {e:#}"),
        }
    }
}

fn warn_unknown_words<'a>(words: &[Vec<u8>], to_check: impl IntoIterator<Item = &'a [u8]>) {
    for word in to_check {
        if words.binary_search_by(|w| w.as_slice().cmp(word)).is_err() {
            eprintln!(
                "Warning: word \"{}\" is not in the wordlist",
                String::from_utf8_lossy(word)
            );
        }
    }
}

fn show_state(guesses: &[Guess], color: bool) {
    if guesses.is_empty() {
        return;
    }
    println!();
    println!("Game so far:");
    for g in guesses {
        println!("  {}", format_guess(g, color));
    }
}

fn pattern_to_string(pattern: &[u8]) -> String {
    pattern
        .iter()
        .map(|&c| match c {
            feedback::GREEN => 'G',
            feedback::YELLOW => 'Y',
            _ => 'X',
        })
        .collect()
}

fn format_guess(g: &Guess, color: bool) -> String {
    let word = String::from_utf8_lossy(&g.word).to_uppercase();
    if !color {
        return format!("{} {}", word, pattern_to_string(&g.pattern));
    }
    let mut out = String::new();
    for (ch, &cell) in word.chars().zip(&g.pattern) {
        match cell {
            feedback::GREEN => out.push_str("\x1b[32m"),
            feedback::YELLOW => out.push_str("\x1b[33m"),
            _ => {
                out.push(ch);
                continue;
            }
        }
        out.push(ch);
        out.push_str("\x1b[0m");
    }
    out
}

/// Print the report for the current guesses: solved/contradiction/single-
/// candidate outcomes, or the candidate count and suggestion sections.
/// Returns true when the game is over.
fn report(
    words: &[Vec<u8>],
    answer_pool: &[Vec<u8>],
    guesses: &[Guess],
    queries: &[Vec<u8>],
    cli: &Cli,
    word_len: usize,
) -> Result<bool> {
    if let Some(solved) = guesses
        .iter()
        .find(|g| g.pattern.iter().all(|&c| c == feedback::GREEN))
    {
        println!("Solved: {}", String::from_utf8_lossy(&solved.word));
        return Ok(true);
    }

    let candidates = solver::filter_candidates(answer_pool, guesses);
    match candidates.len() {
        0 => bail!(
            "No candidates match these guesses. Check the patterns for typos or a wordlist mismatch."
        ),
        1 => {
            println!(
                "Only one candidate remains: {}",
                String::from_utf8_lossy(candidates[0])
            );
            return Ok(true);
        }
        _ => {}
    }

    println!(
        "Candidates remaining: {} of {}",
        candidates.len(),
        answer_pool.len()
    );

    let full_pool: Vec<&[u8]>;
    let pool: &[&[u8]] = if cli.candidates_only {
        &candidates
    } else {
        full_pool = words.iter().map(Vec::as_slice).collect();
        &full_pool
    };

    let scored = solver::score_guesses(pool, &candidates, word_len);

    for query in queries {
        let s = solver::score_guesses(&[query.as_slice()], &candidates, word_len)
            .pop()
            .expect("scoring one query yields one suggestion");
        let (bits_rank, bits_ties) = rank_among(&scored, &s, |t| t.bits);
        let (win_rank, win_ties) = rank_among(&scored, &s, |t| t.win_prob);
        println!();
        println!("Query {}:", s.word.to_uppercase());
        println!(
            "  {:>5.2} bits   rank {bits_rank} of {} by information{}",
            s.bits,
            scored.len(),
            tie_clause(bits_ties)
        );
        println!(
            "  {:>5.1}% win   rank {win_rank} of {} by win probability{}",
            100.0 * s.win_prob,
            scored.len(),
            tie_clause(win_ties)
        );
        println!(
            "  {}",
            if s.is_candidate {
                "still a possible answer"
            } else {
                "not a possible answer"
            }
        );
    }

    let subject = if cli.candidates_only {
        "candidates"
    } else {
        "guesses"
    };
    let by_info = solver::top_by_information(&scored, cli.top);
    let by_win = solver::top_by_win_probability(&scored, cli.top);

    print_section(&format!("Top {subject} by expected information"), &by_info);
    if !cli.candidates_only {
        print_section(
            "Top candidates by expected information",
            &solver::top_by_information(scored.iter().filter(|s| s.is_candidate), cli.top),
        );
    }
    let win_title = format!("Top {subject} by win probability (this guess or the next)");
    if same_words(&by_info, &by_win) {
        if !by_win.is_empty() {
            println!();
            println!("{win_title}: same as by expected information");
        }
    } else {
        print_section(&win_title, &by_win);
    }
    if !cli.candidates_only && cli.top > 0 {
        println!();
        println!("* = still a possible answer");
    }
    Ok(false)
}

fn same_words(a: &[&solver::Suggestion], b: &[&solver::Suggestion]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.word == y.word)
}

/// Competition rank of `query` within `scored` under `key` (1 = best; tied
/// words share a rank), plus how many other words have exactly the same score.
fn rank_among(
    scored: &[solver::Suggestion],
    query: &solver::Suggestion,
    key: impl Fn(&solver::Suggestion) -> f64,
) -> (usize, usize) {
    let value = key(query);
    let ahead = scored.iter().filter(|s| key(s) > value).count();
    let tied = scored
        .iter()
        .filter(|s| key(s) == value && s.word != query.word)
        .count();
    (ahead + 1, tied)
}

fn tie_clause(tied: usize) -> String {
    match tied {
        0 => String::new(),
        1 => " (tied with 1 other)".to_string(),
        n => format!(" (tied with {n} others)"),
    }
}

fn print_section(title: &str, suggestions: &[&solver::Suggestion]) {
    if suggestions.is_empty() {
        return;
    }
    println!();
    println!("{title}:");
    let width = suggestions.iter().map(|s| s.word.len()).max().unwrap_or(0);
    for (i, s) in suggestions.iter().enumerate() {
        let marker = if s.is_candidate { "   *" } else { "" };
        println!(
            "{:>4}. {:<width$}   {:>5.2} bits   {:>5.1}% win{marker}",
            i + 1,
            s.word,
            s.bits,
            100.0 * s.win_prob
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use feedback::{GRAY, GREEN, YELLOW};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parse_guess_happy_path() {
        let g = parse_guess("GREEN", "GXYYX", 5).unwrap();
        assert_eq!(g.word, b"green");
        assert_eq!(g.pattern, [GREEN, GRAY, YELLOW, YELLOW, GRAY]);
    }

    #[test]
    fn parse_guesses_accepts_quoted_pairs() {
        let guesses = parse_guesses(&args(&["TARES XXGYY", "MELEE XXGGX"]), 5).unwrap();
        assert_eq!(guesses.len(), 2);
        assert_eq!(guesses[0].word, b"tares");
        assert_eq!(guesses[1].word, b"melee");
    }

    #[test]
    fn parse_guesses_accepts_unquoted_tokens() {
        let guesses = parse_guesses(&args(&["TARES", "XXGYY", "MELEE", "XXGGX"]), 5).unwrap();
        assert_eq!(guesses.len(), 2);
        assert_eq!(guesses[0].word, b"tares");
        assert_eq!(guesses[1].word, b"melee");
    }

    #[test]
    fn parse_guesses_accepts_mixed_quoting() {
        let guesses = parse_guesses(&args(&["TARES XXGYY", "MELEE", "XXGGX"]), 5).unwrap();
        assert_eq!(guesses.len(), 2);
        assert_eq!(guesses[1].word, b"melee");
        assert_eq!(guesses[1].pattern, [GRAY, GRAY, GREEN, GREEN, GRAY]);
    }

    #[test]
    fn parse_guesses_rejects_odd_token_count() {
        let err = parse_guesses(&args(&["tares XXGYY melee"]), 5)
            .unwrap_err()
            .to_string();
        assert!(err.contains("odd number of tokens (3)"), "{err}");
    }

    #[test]
    fn parse_guesses_rejects_wrong_word_length() {
        let err = parse_guesses(&args(&["greens GXYYXG"]), 5)
            .unwrap_err()
            .to_string();
        assert!(err.contains("guess 1"), "{err}");
        let err = parse_guess("greens", "GXYYXG", 5).unwrap_err().to_string();
        assert!(err.contains("has length 6, expected 5"), "{err}");
    }

    #[test]
    fn parse_guess_rejects_wrong_pattern_length() {
        let err = parse_guess("green", "GXYY", 5).unwrap_err().to_string();
        assert!(err.contains("has length 4, expected 5"), "{err}");
    }

    #[test]
    fn parse_guess_rejects_non_alphabetic_word() {
        let err = parse_guess("gr33n", "GXYYX", 5).unwrap_err().to_string();
        assert!(err.contains("non-alphabetic"), "{err}");
    }

    #[test]
    fn format_guess_plain_matches_input_format() {
        let g = parse_guess("stint", "GGXXG", 5).unwrap();
        assert_eq!(format_guess(&g, false), "STINT GGXXG");
    }

    #[test]
    fn format_guess_colored_wraps_green_and_yellow_letters() {
        let g = parse_guess("taros", "YXXYY", 5).unwrap();
        assert_eq!(
            format_guess(&g, true),
            "\x1b[33mT\x1b[0mAR\x1b[33mO\x1b[0m\x1b[33mS\x1b[0m"
        );
        let g = parse_guess("stint", "GGXXG", 5).unwrap();
        assert_eq!(
            format_guess(&g, true),
            "\x1b[32mS\x1b[0m\x1b[32mT\x1b[0mIN\x1b[32mT\x1b[0m"
        );
    }

    #[test]
    fn pattern_round_trips_through_string_form() {
        let pattern = feedback::parse_pattern("GXYYX", 5).unwrap();
        assert_eq!(pattern_to_string(&pattern), "GXYYX");
    }

    fn suggestion(word: &str, bits: f64) -> solver::Suggestion {
        solver::Suggestion {
            word: word.to_string(),
            bits,
            win_prob: 0.0,
            is_candidate: false,
        }
    }

    #[test]
    fn rank_among_uses_competition_ranking() {
        let scored = vec![
            suggestion("aaaaa", 3.0),
            suggestion("bbbbb", 2.0),
            suggestion("ccccc", 2.0),
            suggestion("ddddd", 1.0),
        ];
        // A query tied with the leaders ranks 1st.
        assert_eq!(
            rank_among(&scored, &suggestion("query", 3.0), |s| s.bits),
            (1, 1)
        );
        // Tied with bbbbb/ccccc: one word ahead, two tied.
        assert_eq!(
            rank_among(&scored, &suggestion("query", 2.0), |s| s.bits),
            (2, 2)
        );
        // A query already in the pool does not count itself as a tie.
        assert_eq!(
            rank_among(&scored, &suggestion("ddddd", 1.0), |s| s.bits),
            (4, 0)
        );
        // Worse than everything.
        assert_eq!(
            rank_among(&scored, &suggestion("query", 0.5), |s| s.bits),
            (5, 0)
        );
    }

    #[test]
    fn same_words_compares_words_in_order() {
        let a = suggestion("aaaaa", 3.0);
        let b = suggestion("bbbbb", 2.0);
        assert!(same_words(&[&a, &b], &[&a, &b]));
        assert!(!same_words(&[&a, &b], &[&b, &a]));
        assert!(!same_words(&[&a, &b], &[&a]));
        assert!(same_words(&[], &[]));
    }

    #[test]
    fn tie_clause_wording() {
        assert_eq!(tie_clause(0), "");
        assert_eq!(tie_clause(1), " (tied with 1 other)");
        assert_eq!(tie_clause(3), " (tied with 3 others)");
    }
}
