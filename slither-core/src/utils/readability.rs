/// Computes the Flesch-Kincaid Reading Ease score for the given text.
///
/// Formula: 206.835 - 1.015 * (words/sentences) - 84.6 * (syllables/words)
///
/// Returns `None` for empty text or text with zero words.
/// The result is clamped to the range 0.0..=100.0.
pub fn flesch_kincaid_reading_ease(text: &str) -> Option<f32> {
    let words = count_words(text);
    if words == 0 {
        return None;
    }

    // Flesch is defined over English prose. Applied outside that domain it does
    // not fail loudly — it returns a clamp bound that reads like a measurement.
    // Every Korean page in one sample scored exactly 100.0 (Hangul words count
    // as one syllable each, hitting the ceiling) and every page of a Japanese
    // link index scored exactly 0.0. Return None instead so the metric is
    // absent rather than confidently wrong.
    if !is_predominantly_latin(text) {
        return None;
    }

    let sentences = count_sentences(text);

    // Whole-page text includes navigation and footer link text, which carries no
    // terminal punctuation. A megamenu collapsed to one 4,371-word "sentence"
    // and scored 0.0 — "Very Difficult" — for the same prose that scored 90.3
    // without the menu. Below roughly one sentence per 40 words the input is a
    // link index, not prose, and the ratio term is meaningless.
    const MAX_WORDS_PER_SENTENCE: usize = 40;
    if words / sentences.max(1) > MAX_WORDS_PER_SENTENCE {
        return None;
    }

    let words_f = words as f32;
    let sentences_f = sentences as f32;
    let syllables_f = count_syllables_in_text(text) as f32;

    let score = 206.835 - 1.015 * (words_f / sentences_f) - 84.6 * (syllables_f / words_f);

    Some(score.clamp(0.0, 100.0))
}

/// True when the text is mostly Latin-script letters, the domain Flesch covers.
fn is_predominantly_latin(text: &str) -> bool {
    let mut latin = 0usize;
    let mut letters = 0usize;
    for c in text.chars().filter(|c| c.is_alphabetic()) {
        letters += 1;
        if c.is_ascii_alphabetic() || matches!(c as u32, 0x00C0..=0x024F) {
            latin += 1;
        }
    }
    // Require a Latin majority. The small floor only guards against
    // near-empty text; a CJK page carrying a few Latin words fails the majority
    // test regardless, since its letter count is dominated by CJK.
    letters >= 10 && latin * 2 >= letters
}

/// Counts the number of sentences in the text by counting sentence-ending
/// punctuation (`.`, `!`, `?`), ignoring consecutive terminators.
///
/// If no terminators are found, returns 1 (treating the text as a single sentence).
pub fn count_sentences(text: &str) -> usize {
    let mut count = 0;
    let mut prev_was_terminator = false;

    for c in text.chars() {
        if c == '.' || c == '!' || c == '?' {
            if !prev_was_terminator {
                count += 1;
            }
            prev_was_terminator = true;
        } else {
            prev_was_terminator = false;
        }
    }

    if count == 0 {
        1
    } else {
        count
    }
}

/// Counts the number of words by splitting on whitespace.
pub fn count_words(text: &str) -> usize {
    text.split_whitespace().count()
}

/// Counts the total number of syllables across all words in the text.
pub fn count_syllables_in_text(text: &str) -> usize {
    text.split_whitespace().map(count_syllables_in_word).sum()
}

/// Estimates the number of syllables in a single word using vowel group counting.
///
/// Algorithm:
/// 1. Strip non-alphabetic characters and convert to lowercase.
/// 2. If the cleaned word is 3 characters or fewer, return 1.
/// 3. Count groups of consecutive vowels (a, e, i, o, u, y).
/// 4. Subtract 1 if the word ends with a silent trailing 'e'.
/// 5. Return a minimum of 1.
pub fn count_syllables_in_word(word: &str) -> usize {
    let cleaned: String = word
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| c.to_ascii_lowercase())
        .collect();

    if cleaned.is_empty() {
        return 0;
    }

    if cleaned.len() <= 3 {
        return 1;
    }

    let mut count: usize = 0;
    let mut prev_was_vowel = false;

    for c in cleaned.chars() {
        let is_vowel = matches!(c, 'a' | 'e' | 'i' | 'o' | 'u' | 'y');
        if is_vowel && !prev_was_vowel {
            count += 1;
        }
        prev_was_vowel = is_vowel;
    }

    // Subtract 1 for silent trailing 'e'
    if cleaned.ends_with('e') {
        count = count.saturating_sub(1);
    }

    // Minimum 1 syllable
    count.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_text_returns_none() {
        assert_eq!(flesch_kincaid_reading_ease(""), None);
    }

    #[test]
    fn test_whitespace_only_returns_none() {
        assert_eq!(flesch_kincaid_reading_ease("   "), None);
    }

    #[test]
    fn test_simple_text_scores_high() {
        // Simple, easy-to-read text should score above 60
        let score = flesch_kincaid_reading_ease("The cat sat on the mat.").unwrap();
        assert!(
            score > 60.0,
            "Simple text should score above 60 (easy), got {score}"
        );
    }

    #[test]
    fn test_complex_text_scores_low() {
        // Complex academic text with long words should score below 30
        let complex = "The circumnavigation of epistemological methodologies \
            necessitates a comprehensive understanding of interdisciplinary \
            conceptualization and the juxtaposition of phenomenological paradigms.";
        let score = flesch_kincaid_reading_ease(complex).unwrap();
        assert!(
            score < 30.0,
            "Complex text should score below 30 (difficult), got {score}"
        );
    }

    #[test]
    fn test_syllable_count_cat() {
        assert_eq!(count_syllables_in_word("cat"), 1);
    }

    #[test]
    fn test_syllable_count_hello() {
        assert_eq!(count_syllables_in_word("hello"), 2);
    }

    #[test]
    fn test_syllable_count_beautiful() {
        assert_eq!(count_syllables_in_word("beautiful"), 3);
    }

    #[test]
    fn test_count_sentences_basic() {
        assert_eq!(count_sentences("Hello. World. Test."), 3);
    }

    #[test]
    fn test_count_sentences_consecutive_terminators() {
        // "Wait...what?!" has consecutive terminators that should not be double-counted
        assert_eq!(count_sentences("Wait...what?!"), 2);
    }

    #[test]
    fn test_count_sentences_no_terminators() {
        // Text with no sentence terminators is treated as 1 sentence
        assert_eq!(count_sentences("No terminator here"), 1);
    }

    #[test]
    fn test_count_words() {
        assert_eq!(count_words("The cat sat on the mat"), 6);
    }

    #[test]
    fn test_count_words_empty() {
        assert_eq!(count_words(""), 0);
    }

    #[test]
    fn test_syllables_empty_word() {
        assert_eq!(count_syllables_in_word(""), 0);
    }

    #[test]
    fn test_syllables_with_punctuation() {
        // "hello," should still count as 2 syllables
        assert_eq!(count_syllables_in_word("hello,"), 2);
    }

    #[test]
    fn test_score_clamped_to_range() {
        // Any score produced must be inside the Flesch range.
        let score = flesch_kincaid_reading_ease("The quick brown fox jumps over the lazy dog.")
            .expect("ordinary English prose is scorable");
        assert!(
            (0.0..=100.0).contains(&score),
            "Score should be clamped to 0-100, got {score}"
        );
    }

    /// The metric is only meaningful over English prose, so text too short to
    /// judge now returns None rather than a clamp bound that reads like a
    /// measurement. (Previously "Word." scored 100.0 — "very easy to read".)
    #[test]
    fn text_too_short_to_judge_has_no_score() {
        assert_eq!(flesch_kincaid_reading_ease("Word."), None);
        assert_eq!(flesch_kincaid_reading_ease(""), None);
    }
}
