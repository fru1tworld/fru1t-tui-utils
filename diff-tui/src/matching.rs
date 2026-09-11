use std::{ops::Range, time::Instant};

use similar::{Algorithm, DiffTag, capture_diff_slices_deadline};
use unicode_segmentation::UnicodeSegmentation;

use crate::wrap::token_ranges;

pub fn changed_words(
    before: &str,
    after: &str,
    deadline: Instant,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let old_ranges = token_ranges(before);
    let new_ranges = token_ranges(after);
    let old: Vec<_> = old_ranges
        .iter()
        .map(|range| &before[range.clone()])
        .collect();
    let new: Vec<_> = new_ranges
        .iter()
        .map(|range| &after[range.clone()])
        .collect();
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for op in capture_diff_slices_deadline(Algorithm::Myers, &old, &new, Some(deadline)) {
        let (tag, old, new) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        if !old.is_empty() {
            removed.push(old_ranges[old.start].start..old_ranges[old.end - 1].end);
        }
        if !new.is_empty() {
            added.push(new_ranges[new.start].start..new_ranges[new.end - 1].end);
        }
    }
    (removed, added)
}

pub fn align_lines(
    before: &[&str],
    after: &[&str],
    deadline: Instant,
) -> Vec<(Option<usize>, Option<usize>)> {
    let positional = || {
        (0..before.len().max(after.len()))
            .map(|index| {
                (
                    (index < before.len()).then_some(index),
                    (index < after.len()).then_some(index),
                )
            })
            .collect()
    };
    // Bound alignment work for generated files and large replacements.
    if before.is_empty()
        || after.is_empty()
        || (before.len() == 1 && after.len() == 1)
        || before.len().saturating_mul(after.len()) > 4096
        || Instant::now() >= deadline
    {
        return positional();
    }
    let old: Vec<Vec<_>> = before
        .iter()
        .map(|line| {
            line.graphemes(true)
                .filter(|s| !s.chars().all(char::is_whitespace))
                .collect()
        })
        .collect();
    let new: Vec<Vec<_>> = after
        .iter()
        .map(|line| {
            line.graphemes(true)
                .filter(|s| !s.chars().all(char::is_whitespace))
                .collect()
        })
        .collect();
    let columns = after.len() + 1;
    let mut scores = vec![0.0_f32; (before.len() + 1) * columns];
    let mut paired = vec![false; scores.len()];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            if Instant::now() >= deadline {
                return positional();
            }
            let total = old[i].len() + new[j].len();
            let equal: usize =
                capture_diff_slices_deadline(Algorithm::Myers, &old[i], &new[j], Some(deadline))
                    .iter()
                    .map(|op| {
                        let (tag, range, _) = op.as_tag_tuple();
                        if tag == DiffTag::Equal {
                            range.len()
                        } else {
                            0
                        }
                    })
                    .sum();
            let ratio = if total == 0 {
                1.0
            } else {
                2.0 * equal as f32 / total as f32
            };
            let at = i * columns + j;
            let skip = scores[at + columns].max(scores[at + 1]);
            let pair = ratio * ratio + scores[at + columns + 1];
            paired[at] = ratio >= 0.5 && pair >= skip;
            scores[at] = if paired[at] { pair } else { skip };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < before.len() || j < after.len() {
        let at = i * columns + j;
        if i < before.len() && j < after.len() && paired[at] {
            pairs.push((Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if i < before.len() && (j == after.len() || scores[at + columns] >= scores[at + 1]) {
            pairs.push((Some(i), None));
            i += 1;
        } else {
            pairs.push((None, Some(j)));
            j += 1;
        }
    }
    pairs
}
