use textwrap::{Options, WordSeparator, WordSplitter};

/// Wraps `text` to `width` columns, keeping each source line and Hangul words intact.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let keep_hangul_words_whole = WordSeparator::AsciiSpace;
    let options = Options::new(width.max(1))
        .word_separator(keep_hangul_words_whole)
        .word_splitter(WordSplitter::NoHyphenation);
    text.split('\n')
        .flat_map(|line| {
            let chunks = textwrap::wrap(line.trim_end_matches('\r'), &options);
            if chunks.is_empty() {
                vec![String::new()]
            } else {
                chunks.into_iter().map(|chunk| chunk.into_owned()).collect()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_source_lines_and_blank_lines() {
        assert_eq!(wrap("첫 줄\r\n\n- 둘째", 20), vec!["첫 줄", "", "- 둘째"]);
    }
}
