use anyhow::{Result, bail, ensure};

pub const GRAY: u8 = 0;
pub const YELLOW: u8 = 1;
pub const GREEN: u8 = 2;

pub const MAX_WORD_LEN: usize = 40;

/// Compute Wordle feedback for `guess` against `answer` into `out`.
pub fn compute_feedback(guess: &[u8], answer: &[u8], out: &mut [u8]) {
    debug_assert_eq!(guess.len(), answer.len());
    debug_assert_eq!(guess.len(), out.len());

    let mut counts = [0u8; 26];
    for ((&g, &a), o) in guess.iter().zip(answer).zip(out.iter_mut()) {
        if g == a {
            *o = GREEN;
        } else {
            *o = GRAY;
            counts[(a - b'a') as usize] += 1;
        }
    }
    for (&g, o) in guess.iter().zip(out.iter_mut()) {
        if *o == GREEN {
            continue;
        }
        let count = &mut counts[(g - b'a') as usize];
        if *count > 0 {
            *o = YELLOW;
            *count -= 1;
        }
    }
}

/// Parse a pattern string like "GXYYX" (case-insensitive) into cell values.
pub fn parse_pattern(s: &str, expected_len: usize) -> Result<Vec<u8>> {
    let len = s.chars().count();
    ensure!(
        len == expected_len,
        "pattern \"{s}\" has length {len}, expected {expected_len}"
    );
    s.chars()
        .map(|c| match c.to_ascii_uppercase() {
            'G' => Ok(GREEN),
            'Y' => Ok(YELLOW),
            'X' => Ok(GRAY),
            other => bail!("invalid pattern character '{other}' in \"{s}\" (expected G, Y, or X)"),
        })
        .collect()
}

/// Base-3 id of a pattern: sum of cell[i] * 3^i. Requires len <= MAX_WORD_LEN.
pub fn pattern_id(pattern: &[u8]) -> u64 {
    pattern
        .iter()
        .rev()
        .fold(0, |acc, &c| acc * 3 + u64::from(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fb(guess: &str, answer: &str) -> String {
        let mut out = vec![0; guess.len()];
        compute_feedback(guess.as_bytes(), answer.as_bytes(), &mut out);
        out.iter()
            .map(|&c| match c {
                GREEN => 'G',
                YELLOW => 'Y',
                _ => 'X',
            })
            .collect()
    }

    #[test]
    fn exact_match_is_all_green() {
        assert_eq!(fb("crane", "crane"), "GGGGG");
    }

    #[test]
    fn no_overlap_is_all_gray() {
        assert_eq!(fb("crane", "stump"), "XXXXX");
    }

    #[test]
    fn non_five_letter_words() {
        assert_eq!(fb("acre", "race"), "YYYG");
    }

    #[test]
    fn duplicate_in_guess_consumed_by_green() {
        // The answer's only 'a' is claimed by the green, so the second 'a' is gray.
        assert_eq!(fb("algae", "abbey"), "GXXXY");
    }

    #[test]
    fn duplicate_in_guess_single_in_answer() {
        // Only the first 'e' turns yellow; the answer has just one 'e'.
        assert_eq!(fb("keeps", "abbey"), "XYXXX");
    }

    #[test]
    fn green_claims_count_before_yellows() {
        // The answer's single 'e' is claimed by the position-4 green,
        // so the leading 'e's stay gray rather than turning yellow.
        assert_eq!(fb("eerie", "crane"), "XXYXG");
    }

    #[test]
    fn duplicates_in_answer() {
        assert_eq!(fb("babes", "abbey"), "YYGGX");
    }

    #[test]
    fn duplicates_in_both() {
        assert_eq!(fb("speed", "erase"), "YXYYX");
    }

    #[test]
    fn parse_pattern_is_case_insensitive() {
        assert_eq!(
            parse_pattern("gXyYx", 5).unwrap(),
            vec![GREEN, GRAY, YELLOW, YELLOW, GRAY]
        );
    }

    #[test]
    fn parse_pattern_rejects_bad_char() {
        let err = parse_pattern("GXZYX", 5).unwrap_err().to_string();
        assert!(err.contains("invalid pattern character 'Z'"), "{err}");
    }

    #[test]
    fn parse_pattern_rejects_wrong_length() {
        let err = parse_pattern("GXY", 5).unwrap_err().to_string();
        assert!(err.contains("has length 3, expected 5"), "{err}");
    }

    #[test]
    fn pattern_id_bounds() {
        assert_eq!(pattern_id(&[GRAY; 5]), 0);
        assert_eq!(pattern_id(&[GREEN; 5]), 242);
        // Distinct patterns get distinct ids.
        assert_ne!(
            pattern_id(&[YELLOW, GRAY, GRAY, GRAY, GRAY]),
            pattern_id(&[GRAY, YELLOW, GRAY, GRAY, GRAY])
        );
        // Max supported length must not overflow (would panic in debug builds).
        assert_eq!(
            pattern_id(&[GREEN; MAX_WORD_LEN]),
            3u64.pow(MAX_WORD_LEN as u32) - 1
        );
    }
}
