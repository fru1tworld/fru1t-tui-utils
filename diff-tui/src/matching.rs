use std::{ops::Range, time::Instant};

use similar::{Algorithm, DiffTag, capture_diff_slices_deadline};
use unicode_segmentation::UnicodeSegmentation;

use crate::wrap::lexeme_ranges;

const MAX_ALIGNMENT_CELLS: usize = 4096;
const MIN_PAIR_SIMILARITY: f32 = 0.5;

pub fn changed_words(
    before: &str,
    after: &str,
    deadline: Instant,
) -> (Vec<Range<usize>>, Vec<Range<usize>>) {
    let old_ranges = lexeme_ranges(before);
    let new_ranges = lexeme_ranges(after);
    let old = slices(before, &old_ranges);
    let new = slices(after, &new_ranges);
    let byte_span = |ranges: &[Range<usize>], lexemes: Range<usize>| {
        ranges[lexemes.start].start..ranges[lexemes.end - 1].end
    };
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for op in capture_diff_slices_deadline(Algorithm::Myers, &old, &new, Some(deadline)) {
        let (tag, old_lexemes, new_lexemes) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            continue;
        }
        if !old_lexemes.is_empty() {
            removed.push(byte_span(&old_ranges, old_lexemes));
        }
        if !new_lexemes.is_empty() {
            added.push(byte_span(&new_ranges, new_lexemes));
        }
    }
    (removed, added)
}

fn slices<'a>(text: &'a str, ranges: &[Range<usize>]) -> Vec<&'a str> {
    ranges.iter().map(|range| &text[range.clone()]).collect()
}

pub fn align_lines(
    before: &[&str],
    after: &[&str],
    deadline: Instant,
) -> Vec<(Option<usize>, Option<usize>)> {
    let positional = || positional_pairs(before.len(), after.len());
    let too_large = before.len().saturating_mul(after.len()) > MAX_ALIGNMENT_CELLS;
    let single_replacement = before.len() == 1 && after.len() == 1;
    if before.is_empty()
        || after.is_empty()
        || single_replacement
        || too_large
        || Instant::now() >= deadline
    {
        return positional();
    }
    let old: Vec<_> = before
        .iter()
        .map(|line| significant_graphemes(line))
        .collect();
    let new: Vec<_> = after
        .iter()
        .map(|line| significant_graphemes(line))
        .collect();
    let columns = after.len() + 1;
    let mut scores = vec![0.0_f32; (before.len() + 1) * columns];
    let mut pair_here = vec![false; scores.len()];
    for i in (0..before.len()).rev() {
        for j in (0..after.len()).rev() {
            if Instant::now() >= deadline {
                return positional();
            }
            let ratio = similarity(&old[i], &new[j], deadline);
            let cell = i * columns + j;
            let without_pair = scores[cell + columns].max(scores[cell + 1]);
            let with_pair = ratio * ratio + scores[cell + columns + 1];
            pair_here[cell] = ratio >= MIN_PAIR_SIMILARITY && with_pair >= without_pair;
            scores[cell] = if pair_here[cell] {
                with_pair
            } else {
                without_pair
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < before.len() || j < after.len() {
        let cell = i * columns + j;
        if i < before.len() && j < after.len() && pair_here[cell] {
            pairs.push((Some(i), Some(j)));
            i += 1;
            j += 1;
        } else if i < before.len()
            && (j == after.len() || scores[cell + columns] >= scores[cell + 1])
        {
            pairs.push((Some(i), None));
            i += 1;
        } else {
            pairs.push((None, Some(j)));
            j += 1;
        }
    }
    pairs
}

fn positional_pairs(before: usize, after: usize) -> Vec<(Option<usize>, Option<usize>)> {
    (0..before.max(after))
        .map(|index| {
            (
                (index < before).then_some(index),
                (index < after).then_some(index),
            )
        })
        .collect()
}

fn significant_graphemes(line: &str) -> Vec<&str> {
    line.graphemes(true)
        .filter(|grapheme| !grapheme.chars().all(char::is_whitespace))
        .collect()
}

fn similarity(old: &[&str], new: &[&str], deadline: Instant) -> f32 {
    let total = old.len() + new.len();
    if total == 0 {
        return 1.0;
    }
    let equal: usize = capture_diff_slices_deadline(Algorithm::Myers, old, new, Some(deadline))
        .iter()
        .filter_map(|op| {
            let (tag, range, _) = op.as_tag_tuple();
            (tag == DiffTag::Equal).then_some(range.len())
        })
        .sum();
    2.0 * equal as f32 / total as f32
}
