//! Suggestion engine: nearest-match lookup used to power corrective error
//! messages such as `unknown notion 'invoice'; closest is 'Invoice'`.

/// Levenshtein (edit) distance between `a` and `b`: the minimum number of
/// single-character insertions, deletions, or substitutions to turn one
/// into the other.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (rows, cols) = (a.len(), b.len());

    let mut dp = vec![vec![0usize; cols + 1]; rows + 1];
    for (i, row) in dp.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in dp[0].iter_mut().enumerate() {
        *cell = j;
    }

    for i in 1..=rows {
        for j in 1..=cols {
            let substitution_cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + substitution_cost);
        }
    }

    dp[rows][cols]
}

/// Finds the candidate closest to `target`, for use in "did you mean ...?"
/// diagnostics.
///
/// A candidate is eligible if its edit distance to `target` is at most
/// `max(1, target.len() / 3)`, or if it is equal to `target` up to case.
/// Among eligible candidates, the one with the smallest edit distance wins;
/// ties are broken by picking the lexicographically smallest candidate.
pub fn closest<'a, I>(target: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let threshold = (target.len() / 3).max(1);
    let target_lower = target.to_lowercase();

    let mut best: Option<(&'a str, usize)> = None;
    for candidate in candidates {
        let distance = edit_distance(target, candidate);
        let eligible = distance <= threshold || candidate.to_lowercase() == target_lower;
        if !eligible {
            continue;
        }
        best = Some(match best {
            None => (candidate, distance),
            Some((best_candidate, best_distance)) => {
                if distance < best_distance
                    || (distance == best_distance && candidate < best_candidate)
                {
                    (candidate, distance)
                } else {
                    (best_candidate, best_distance)
                }
            }
        });
    }
    best.map(|(candidate, _)| candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_of_identical_strings_is_zero() {
        assert_eq!(edit_distance("abc", "abc"), 0);
    }

    #[test]
    fn edit_distance_of_empty_strings_is_zero() {
        assert_eq!(edit_distance("", ""), 0);
    }

    #[test]
    fn edit_distance_classic_kitten_sitting() {
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn edit_distance_one_insertion() {
        assert_eq!(edit_distance("setled", "settled"), 1);
    }

    #[test]
    fn closest_matches_case_insensitively() {
        assert_eq!(closest("invoice", ["Invoice", "Customer"]), Some("Invoice"));
    }

    #[test]
    fn closest_matches_within_edit_distance_threshold() {
        assert_eq!(
            closest("setled", ["open", "settled", "cancelled"]),
            Some("settled")
        );
    }

    #[test]
    fn closest_returns_none_when_nothing_is_close_enough() {
        assert_eq!(closest("xyz", ["Invoice"]), None);
    }

    #[test]
    fn closest_returns_none_for_no_candidates() {
        let candidates: [&str; 0] = [];
        assert_eq!(closest("anything", candidates), None);
    }

    #[test]
    fn closest_breaks_ties_lexicographically() {
        // "cct" and "cbt" are both at edit distance 1 from "cat"; the
        // lexicographically smaller one ("cbt") must win regardless of
        // input order.
        assert_eq!(closest("cat", ["cct", "cbt"]), Some("cbt"));
        assert_eq!(closest("cat", ["cbt", "cct"]), Some("cbt"));
    }
}
