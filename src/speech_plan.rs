//! Word times for a recognizer that answers with text only (Hex, a command transcriber without
//! words, a Parakeet model without word times): `spread` places the words over the speech they
//! came from, by length. The times are estimates (`TranscriptData::estimated`).

use crate::model::{TranscriptSegment, WordTiming};

/// Spreads `text` over `spans` (in order, not overlapping), each word's share weighted by its
/// length. A span gets the words whose middle falls in its share of the spans' total length,
/// and they tile it exactly, so no word is placed between spans. No spans place nothing.
pub fn spread(text: &str, spans: &[(u64, u64)]) -> (Vec<TranscriptSegment>, Vec<WordTiming>) {
    let (mut segments, mut words) = (Vec::new(), Vec::new());
    if spans.is_empty() {
        return (segments, words);
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let total: u128 = tokens.iter().map(|token| weight(token)).sum();
    let speech: u128 = spans
        .iter()
        .map(|&(start, end)| u128::from(end.saturating_sub(start)))
        .sum();
    let mut groups: Vec<Vec<&str>> = vec![Vec::new(); spans.len()];
    let (mut before, mut span, mut span_end) = (0_u128, 0, 0_u128);
    for token in tokens {
        // Where the word's middle falls in the speech, scaled by 2 × `total` to stay whole.
        let middle = (2 * before + weight(token)) * speech;
        before += weight(token);
        while span + 1 < spans.len() {
            let end = span_end + u128::from(spans[span].1.saturating_sub(spans[span].0));
            if middle < 2 * total * end {
                break;
            }
            span_end = end;
            span += 1;
        }
        groups[span].push(token);
    }
    for (&(start, end), group) in spans.iter().zip(groups) {
        spread_one(&group, start, end, &mut segments, &mut words);
    }
    (segments, words)
}

fn weight(token: &str) -> u128 {
    token.chars().count().max(1) as u128
}

fn ends_sentence(word: &str) -> bool {
    word.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
        .ends_with(['.', '!', '?', ';', ':'])
}

/// Spreads `tokens` over `start..end` by length: they tile it exactly (no gaps, no overlap), so
/// the first begins at `start` and the last ends at `end`. A segment ends at each sentence end.
fn spread_one(
    tokens: &[&str],
    start: u64,
    end: u64,
    segments: &mut Vec<TranscriptSegment>,
    words: &mut Vec<WordTiming>,
) {
    let end = end.max(start);
    let total: u128 = tokens.iter().map(|token| weight(token)).sum();
    let mut segment_start = words.len();
    let mut weight_so_far: u128 = 0;
    for (index, token) in tokens.iter().enumerate() {
        let at = |share: u128| start + (u128::from(end - start) * share / total) as u64;
        let t0 = at(weight_so_far);
        weight_so_far += weight(token);
        words.push(WordTiming {
            t0,
            t1: at(weight_so_far),
            text: (*token).to_owned(),
        });
        if ends_sentence(token) || index + 1 == tokens.len() {
            let phrase = &words[segment_start..];
            segments.push(TranscriptSegment {
                t0: phrase[0].t0,
                t1: phrase[phrase.len() - 1].t1,
                text: phrase
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            segment_start = words.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreading_tiles_the_span_by_word_length() {
        let (segments, words) = spread("Hi there. Longer words follow", &[(1_000, 2_000)]);
        assert_eq!(words.first().unwrap().t0, 1_000);
        assert_eq!(words.last().unwrap().t1, 2_000);
        assert!(words.windows(2).all(|pair| pair[0].t1 == pair[1].t0));
        assert!(words.iter().all(|word| word.t0 <= word.t1));
        let length = |i: usize| words[i].t1 - words[i].t0;
        assert!(length(3) > length(0), "longer words get longer spans");
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Hi there.");
        assert_eq!((segments[0].t0, segments[1].t1), (1_000, 2_000));
        assert_eq!(segments[0].t1, words[1].t1);
        assert_eq!(spread("  ", &[(0, 10)]), (vec![], vec![]));
        let (_, instant) = spread("a b", &[(500, 500)]);
        assert!(instant.iter().all(|word| word.t0 == 500 && word.t1 == 500));
    }

    #[test]
    fn spreading_over_spans_never_places_a_word_between_them() {
        let spans = [(1_000, 2_000), (5_000, 6_000), (9_000, 9_500)];
        let (segments, words) = spread("Open the menu. Then click the button now.", &spans);
        assert_eq!(words.len(), 8);
        assert!(words.iter().all(|word| {
            spans
                .iter()
                .any(|&(start, end)| word.t0 >= start && word.t1 <= end)
        }));
        assert!(words.windows(2).all(|pair| pair[0].t1 <= pair[1].t0));
        assert_eq!((words[0].t0, words[7].t1), (1_000, 9_500));
        assert!(segments.iter().all(|segment| segment.t0 <= segment.t1));
        // Two spans of equal length take half the words each.
        let (_, halves) = spread("aa bb cc dd", &[(0, 100), (1_000, 1_100)]);
        assert!(halves[..2].iter().all(|word| word.t1 <= 100));
        assert!(halves[2..].iter().all(|word| word.t0 >= 1_000));
        assert_eq!(spread("Hi.", &[]), (vec![], vec![]));
        assert_eq!(spread("", &spans), (vec![], vec![]));
    }
}
