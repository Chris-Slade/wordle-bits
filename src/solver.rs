use std::collections::{HashMap, HashSet};

use crate::feedback::{MAX_WORD_LEN, compute_feedback, pattern_id};

#[derive(Debug)]
pub struct Guess {
    pub word: Vec<u8>,
    pub pattern: Vec<u8>,
}

/// A candidate survives iff, treating it as the answer, every past guess
/// reproduces exactly the pattern that was observed.
pub fn filter_candidates<'a>(words: &'a [Vec<u8>], guesses: &[Guess]) -> Vec<&'a [u8]> {
    let mut fb = [0u8; MAX_WORD_LEN];
    words
        .iter()
        .map(Vec::as_slice)
        .filter(|candidate| {
            guesses.iter().all(|g| {
                let fb = &mut fb[..g.word.len()];
                compute_feedback(&g.word, candidate, fb);
                *fb == *g.pattern
            })
        })
        .collect()
}

/// Histogram of feedback-pattern ids, reused across the ranking loop.
///
/// Flat vector when 3^word_len is small enough (cleared via a touched-id list
/// so clearing stays cheap even for large bucket arrays); hash map otherwise.
pub enum Histogram {
    Flat {
        buckets: Vec<u32>,
        touched: Vec<u64>,
    },
    Sparse(HashMap<u64, u32>),
}

const FLAT_LEN_LIMIT: usize = 12; // 3^12 u32 buckets ≈ 2 MB

impl Histogram {
    pub fn new(word_len: usize) -> Self {
        if word_len <= FLAT_LEN_LIMIT {
            Self::Flat {
                buckets: vec![0; 3usize.pow(word_len as u32)],
                touched: Vec::new(),
            }
        } else {
            Self::Sparse(HashMap::new())
        }
    }

    fn clear(&mut self) {
        match self {
            Self::Flat { buckets, touched } => {
                for &id in touched.iter() {
                    buckets[id as usize] = 0;
                }
                touched.clear();
            }
            Self::Sparse(map) => map.clear(),
        }
    }

    fn add(&mut self, id: u64) {
        match self {
            Self::Flat { buckets, touched } => {
                let bucket = &mut buckets[id as usize];
                if *bucket == 0 {
                    touched.push(id);
                }
                *bucket += 1;
            }
            Self::Sparse(map) => *map.entry(id).or_insert(0) += 1,
        }
    }

    fn distinct_patterns(&self) -> usize {
        match self {
            Self::Flat { touched, .. } => touched.len(),
            Self::Sparse(map) => map.len(),
        }
    }

    fn entropy_bits(&self, total: usize) -> f64 {
        let n = total as f64;
        let term = |count: u32| {
            let p = f64::from(count) / n;
            -p * p.log2()
        };
        match self {
            Self::Flat { buckets, touched } => {
                touched.iter().map(|&id| term(buckets[id as usize])).sum()
            }
            Self::Sparse(map) => map.values().map(|&count| term(count)).sum(),
        }
    }
}

/// Expected information (bits) gained by playing `guess` against a uniform
/// distribution over `candidates`.
pub fn entropy_bits(guess: &[u8], candidates: &[&[u8]], hist: &mut Histogram) -> f64 {
    let mut fb = [0u8; MAX_WORD_LEN];
    let fb = &mut fb[..guess.len()];
    hist.clear();
    for candidate in candidates {
        compute_feedback(guess, candidate, fb);
        hist.add(pattern_id(fb));
    }
    hist.entropy_bits(candidates.len())
}

pub struct Suggestion {
    pub word: String,
    pub bits: f64,
    /// Probability of winning within this guess and one follow-up candidate
    /// guess: each feedback pattern's bucket is won next turn with probability
    /// 1/|bucket|, so the total is (distinct patterns) / (candidate count).
    /// A guess that is itself a candidate wins its own bucket immediately.
    pub win_prob: f64,
    pub is_candidate: bool,
}

/// Score every word in `pool` against the remaining candidates. The result is
/// unordered; use the `top_by_*` selectors to rank it.
pub fn score_guesses(pool: &[&[u8]], candidates: &[&[u8]], word_len: usize) -> Vec<Suggestion> {
    let candidate_set: HashSet<&[u8]> = candidates.iter().copied().collect();
    let mut hist = Histogram::new(word_len);
    let n = candidates.len() as f64;
    pool.iter()
        .map(|&word| {
            let bits = entropy_bits(word, candidates, &mut hist);
            Suggestion {
                word: String::from_utf8_lossy(word).into_owned(),
                bits,
                win_prob: hist.distinct_patterns() as f64 / n,
                is_candidate: candidate_set.contains(word),
            }
        })
        .collect()
}

/// Top `top_n` by expected information; ties break toward higher win
/// probability, then possible answers, then alphabetically.
pub fn top_by_information<'a>(
    scored: impl IntoIterator<Item = &'a Suggestion>,
    top_n: usize,
) -> Vec<&'a Suggestion> {
    top_by(scored, top_n, |a, b| {
        b.bits
            .total_cmp(&a.bits)
            .then(b.win_prob.total_cmp(&a.win_prob))
            .then(b.is_candidate.cmp(&a.is_candidate))
            .then(a.word.cmp(&b.word))
    })
}

/// Top `top_n` by win probability; ties break toward higher information,
/// then possible answers, then alphabetically.
pub fn top_by_win_probability<'a>(
    scored: impl IntoIterator<Item = &'a Suggestion>,
    top_n: usize,
) -> Vec<&'a Suggestion> {
    top_by(scored, top_n, |a, b| {
        b.win_prob
            .total_cmp(&a.win_prob)
            .then(b.bits.total_cmp(&a.bits))
            .then(b.is_candidate.cmp(&a.is_candidate))
            .then(a.word.cmp(&b.word))
    })
}

fn top_by<'a>(
    scored: impl IntoIterator<Item = &'a Suggestion>,
    top_n: usize,
    cmp: impl Fn(&Suggestion, &Suggestion) -> std::cmp::Ordering,
) -> Vec<&'a Suggestion> {
    let mut refs: Vec<&Suggestion> = scored.into_iter().collect();
    refs.sort_by(|a, b| cmp(a, b));
    refs.truncate(top_n);
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::feedback::parse_pattern;

    fn words(list: &[&str]) -> Vec<Vec<u8>> {
        list.iter().map(|w| w.as_bytes().to_vec()).collect()
    }

    fn guess(word: &str, pattern: &str) -> Guess {
        Guess {
            word: word.as_bytes().to_vec(),
            pattern: parse_pattern(pattern, word.len()).unwrap(),
        }
    }

    #[test]
    fn filtering_keeps_only_consistent_words() {
        let list = words(&["crane", "brake", "grape", "slate", "cramp"]);
        // Against answer "brake": c gray, r green, a green, n gray, e green.
        let survivors = filter_candidates(&list, &[guess("crane", "XGGXG")]);
        assert_eq!(survivors, [b"brake", b"grape"]);
    }

    #[test]
    fn all_gray_duplicate_guess_letter_means_zero_of_that_letter() {
        let list = words(&["motto", "pinch", "topaz"]);
        // "otter XXXXX" says the answer has no 'o' or 't' at all.
        let survivors = filter_candidates(&list, &[guess("otter", "XXXXX")]);
        assert_eq!(survivors, [b"pinch"]);
    }

    #[test]
    fn contradictory_guesses_leave_nothing() {
        let list = words(&["crane", "slate"]);
        let survivors =
            filter_candidates(&list, &[guess("crane", "GXXXX"), guess("crane", "XXXXX")]);
        assert!(survivors.is_empty());
    }

    #[test]
    fn multiple_guesses_intersect() {
        let list = words(&["brake", "grape", "crane"]);
        let survivors =
            filter_candidates(&list, &[guess("crane", "XGGXG"), guess("brake", "XGGXG")]);
        assert_eq!(survivors, [b"grape"]);
    }

    #[test]
    fn entropy_is_zero_when_all_candidates_share_a_pattern() {
        let list = words(&["azzzz", "zazzz"]);
        let candidates: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
        let mut hist = Histogram::new(5);
        // "qqqqq" shares no letters with either candidate: same all-gray pattern.
        assert_eq!(entropy_bits(b"qqqqq", &candidates, &mut hist), 0.0);
    }

    #[test]
    fn entropy_is_log2_n_when_all_patterns_distinct() {
        let list = words(&["crane", "slate", "grape", "bidet"]);
        let candidates: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
        let mut hist = Histogram::new(5);
        // Guessing a candidate word distinguishes all four (each yields a unique pattern).
        let bits = entropy_bits(b"crane", &candidates, &mut hist);
        assert!((bits - 2.0).abs() < 1e-12, "expected 2 bits, got {bits}");
    }

    #[test]
    fn entropy_of_even_split_is_one_bit() {
        // "azzzz" vs candidates: two contain 'a' at position 0 (green),
        // two contain no letters in common (all gray) — a clean 2/2 split.
        let list = words(&["ammmm", "annnn", "bmmmm", "bnnnn"]);
        let candidates: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
        let mut hist = Histogram::new(5);
        let bits = entropy_bits(b"azzzz", &candidates, &mut hist);
        assert!((bits - 1.0).abs() < 1e-12, "expected 1 bit, got {bits}");
    }

    #[test]
    fn entropy_of_three_one_split() {
        // 'a' green for three candidates, all-gray for one.
        let list = words(&["ammmm", "annnn", "amnmn", "bzzzz"]);
        let candidates: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
        let mut hist = Histogram::new(5);
        let bits = entropy_bits(b"aqqqq", &candidates, &mut hist);
        let expected = -(0.75f64 * 0.75f64.log2() + 0.25 * 0.25f64.log2());
        assert!(
            (bits - expected).abs() < 1e-12,
            "expected {expected}, got {bits}"
        );
    }

    #[test]
    fn ranking_marks_candidates_and_truncates() {
        let list = words(&["ammmm", "annnn", "bmmmm", "bnnnn"]);
        let all: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
        let candidates = vec![all[0], all[1]]; // ammmm, annnn
        let scored = score_guesses(&all, &candidates, 5);
        let ranked = top_by_information(&scored, 2);
        assert_eq!(ranked.len(), 2);
        // Every word in the list distinguishes the two candidates (1 bit each),
        // so the candidate tiebreak puts the two candidates first.
        assert!(ranked[0].is_candidate && ranked[1].is_candidate);
        assert_eq!(ranked[0].word, "ammmm");
    }

    #[test]
    fn candidates_only_pool_restricts_suggestions() {
        let list = words(&["ammmm", "annnn", "bmmmm", "bnnnn"]);
        let all: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
        let candidates = vec![all[0], all[1]];
        let scored = score_guesses(&candidates, &candidates, 5);
        let ranked = top_by_information(&scored, 10);
        assert_eq!(ranked.len(), 2);
        assert!(ranked.iter().all(|s| s.is_candidate));
    }

    #[test]
    fn win_probability_counts_distinct_patterns() {
        // The stout/stopt/stott endgame: each candidate lumps the other two
        // into one bucket (2 patterns -> 2/3), while a word probing the
        // differing letters separates all three (3 patterns -> 1.0).
        let list = words(&["stopt", "stott", "stout"]);
        let candidates: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
        let scored = score_guesses(&candidates, &candidates, 5);
        for s in &scored {
            assert!(
                (s.win_prob - 2.0 / 3.0).abs() < 1e-12,
                "{}: {}",
                s.word,
                s.win_prob
            );
        }
        let probe = score_guesses(&[b"upped"], &candidates, 5);
        assert!(
            (probe[0].win_prob - 1.0).abs() < 1e-12,
            "{}",
            probe[0].win_prob
        );
        assert!(!probe[0].is_candidate);
    }

    #[test]
    fn win_probability_ranking_prefers_full_separation() {
        let list = words(&["stopt", "stott", "stout", "upped"]);
        let all: Vec<&[u8]> = list.iter().map(Vec::as_slice).collect();
        let candidates = vec![all[0], all[1], all[2]];
        let scored = score_guesses(&all, &candidates, 5);
        let by_win = top_by_win_probability(&scored, 4);
        assert_eq!(by_win[0].word, "upped");
        // The information ranking also puts the full separator first here.
        let by_bits = top_by_information(&scored, 4);
        assert_eq!(by_bits[0].word, "upped");
    }
}
