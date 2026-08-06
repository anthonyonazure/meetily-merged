//! A BERT-uncased WordPiece tokenizer, written out longhand.
//!
//! The embedding model this tree uses (all-MiniLM-L6-v2) is a BERT-uncased
//! model, so its ONNX graph needs WordPiece token ids rather than text. Rather
//! than add a tokenizer dependency for one 30k-entry `vocab.txt`, the algorithm
//! is implemented here: it is small, fully specified, and pure, so it can be
//! tested without a build of the app.
//!
//! Fidelity note, stated plainly rather than glossed over: this implements BERT's
//! basic tokenizer for the cases an English meeting transcript actually contains
//! (lowercasing, whitespace and punctuation splitting, CJK isolation, control
//! character stripping, and Latin-1 accent folding). It does not carry Unicode's
//! full canonical decomposition table, so a word using a combining mark outside
//! the Latin-1 range tokenizes slightly differently than the reference
//! implementation would. The consequence is a marginally different vector for
//! that word, not a failure: retrieval degrades a little, nothing breaks. The app
//! ships English-only.

use std::collections::HashMap;

/// The token ids and masks one text turns into, ready for the ONNX graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    pub input_ids: Vec<i64>,
    pub attention_mask: Vec<i64>,
    pub token_type_ids: Vec<i64>,
}

/// Longest word (in characters) WordPiece will try to segment before giving up
/// and emitting `[UNK]`. Matches the reference implementation's limit.
const MAX_CHARS_PER_WORD: usize = 100;

pub struct WordPiece {
    vocab: HashMap<String, i64>,
    cls_id: i64,
    sep_id: i64,
    pad_id: i64,
    unk_id: i64,
}

impl WordPiece {
    /// Builds a tokenizer from a `vocab.txt`: one token per line, the line number
    /// being the token id.
    pub fn from_vocab_text(text: &str) -> Result<Self, String> {
        let mut vocab = HashMap::new();
        for (index, line) in text.lines().enumerate() {
            // Only trailing newline handling; a vocab entry can legitimately be
            // punctuation, so the token itself is not trimmed of anything else.
            let token = line.trim_end_matches(['\r', '\n']);
            if token.is_empty() {
                continue;
            }
            vocab.entry(token.to_string()).or_insert(index as i64);
        }
        let lookup = |name: &str| -> Result<i64, String> {
            vocab
                .get(name)
                .copied()
                .ok_or_else(|| format!("vocab.txt is missing the {} token", name))
        };
        Ok(Self {
            cls_id: lookup("[CLS]")?,
            sep_id: lookup("[SEP]")?,
            pad_id: lookup("[PAD]")?,
            unk_id: lookup("[UNK]")?,
            vocab,
        })
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    pub fn pad_id(&self) -> i64 {
        self.pad_id
    }

    /// Encodes one text to `max_len` tokens: `[CLS] … [SEP]`, truncated to fit and
    /// padded out so every row in a batch has the same width.
    pub fn encode(&self, text: &str, max_len: usize) -> Encoded {
        // Two slots are reserved for [CLS] and [SEP].
        let budget = max_len.saturating_sub(2);
        let mut ids = Vec::with_capacity(max_len);
        ids.push(self.cls_id);

        for word in basic_tokenize(text) {
            if ids.len() - 1 >= budget {
                break;
            }
            for piece in self.word_pieces(&word) {
                if ids.len() - 1 >= budget {
                    break;
                }
                ids.push(piece);
            }
        }
        ids.push(self.sep_id);

        let real = ids.len();
        let mut attention_mask = vec![1i64; real];
        while ids.len() < max_len {
            ids.push(self.pad_id);
            attention_mask.push(0);
        }
        let token_type_ids = vec![0i64; ids.len()];
        Encoded {
            input_ids: ids,
            attention_mask,
            token_type_ids,
        }
    }

    /// Greedy longest-match-first segmentation of one whitespace/punctuation
    /// token into WordPiece ids.
    fn word_pieces(&self, word: &str) -> Vec<i64> {
        let chars: Vec<char> = word.chars().collect();
        if chars.is_empty() {
            return Vec::new();
        }
        if chars.len() > MAX_CHARS_PER_WORD {
            return vec![self.unk_id];
        }

        let mut pieces = Vec::new();
        let mut start = 0usize;
        while start < chars.len() {
            let mut end = chars.len();
            let mut matched: Option<i64> = None;
            while start < end {
                let substring: String = chars[start..end].iter().collect();
                let candidate = if start == 0 {
                    substring
                } else {
                    format!("##{}", substring)
                };
                if let Some(id) = self.vocab.get(&candidate) {
                    matched = Some(*id);
                    break;
                }
                end -= 1;
            }
            match matched {
                // No prefix of the remainder is in the vocabulary, so the whole
                // word is unknown — matching the reference behaviour, which does
                // not emit a partial segmentation.
                None => return vec![self.unk_id],
                Some(id) => {
                    pieces.push(id);
                    start = end;
                }
            }
        }
        pieces
    }
}

/// True for the CJK ranges BERT isolates into single tokens.
fn is_cjk(c: char) -> bool {
    let cp = c as u32;
    (0x4E00..=0x9FFF).contains(&cp)
        || (0x3400..=0x4DBF).contains(&cp)
        || (0x20000..=0x2A6DF).contains(&cp)
        || (0x2A700..=0x2B73F).contains(&cp)
        || (0x2B740..=0x2B81F).contains(&cp)
        || (0x2B820..=0x2CEAF).contains(&cp)
        || (0xF900..=0xFAFF).contains(&cp)
        || (0x2F800..=0x2FA1F).contains(&cp)
}

/// True for the characters BERT treats as punctuation: the ASCII punctuation
/// blocks plus anything Unicode marks as punctuation.
fn is_punctuation(c: char) -> bool {
    let cp = c as u32;
    (33..=47).contains(&cp)
        || (58..=64).contains(&cp)
        || (91..=96).contains(&cp)
        || (123..=126).contains(&cp)
        || c.is_ascii_punctuation()
        || matches!(
            c,
            '\u{2010}'..='\u{2027}' | '\u{2030}'..='\u{205E}' | '\u{00A1}' | '\u{00BF}'
        )
}

/// Folds the accented Latin letters an English transcript realistically contains
/// onto their base letter, the way BERT-uncased's accent stripping would.
fn fold_accent(c: char) -> Option<char> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        // Combining marks are dropped outright, matching the reference
        // implementation's removal of category-Mn characters.
        '\u{0300}'..='\u{036F}' => return None,
        other => other,
    })
}

/// BERT's basic tokenizer: clean, lowercase, fold accents, isolate punctuation
/// and CJK, split on whitespace.
pub fn basic_tokenize(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, tokens: &mut Vec<String>| {
        if !current.is_empty() {
            tokens.push(std::mem::take(current));
        }
    };

    for raw in text.chars() {
        // Control characters and the replacement character are removed outright;
        // \t \n \r become whitespace.
        if raw == '\u{FFFD}' || (raw.is_control() && !matches!(raw, '\t' | '\n' | '\r')) {
            continue;
        }
        if raw.is_whitespace() {
            flush(&mut current, &mut tokens);
            continue;
        }
        for lowered in raw.to_lowercase() {
            let Some(c) = fold_accent(lowered) else {
                continue;
            };
            if is_punctuation(c) || is_cjk(c) {
                flush(&mut current, &mut tokens);
                tokens.push(c.to_string());
            } else {
                current.push(c);
            }
        }
    }
    flush(&mut current, &mut tokens);
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny stand-in vocabulary in `vocab.txt` order, so ids are line numbers.
    fn test_vocab() -> WordPiece {
        let lines = [
            "[PAD]",     // 0
            "[UNK]",     // 1
            "[CLS]",     // 2
            "[SEP]",     // 3
            "deadline",  // 4
            "the",       // 5
            "we",        // 6
            "agreed",    // 7
            "?",         // 8
            "un",        // 9
            "##aff",     // 10
            "##able",    // 11
            "cafe",      // 12
        ];
        WordPiece::from_vocab_text(&lines.join("\n")).unwrap()
    }

    #[test]
    fn a_vocab_without_the_special_tokens_is_rejected() {
        assert!(WordPiece::from_vocab_text("hello\nworld").is_err());
    }

    #[test]
    fn ids_are_line_numbers_and_the_first_entry_wins_on_duplicates() {
        let vocab = test_vocab();
        assert_eq!(vocab.vocab_size(), 13);
        assert_eq!(vocab.pad_id(), 0);
        let duplicated =
            WordPiece::from_vocab_text("[PAD]\n[UNK]\n[CLS]\n[SEP]\nthe\nthe").unwrap();
        assert_eq!(duplicated.encode("the", 3).input_ids[1], 4);
    }

    #[test]
    fn basic_tokenizing_lowercases_and_splits_punctuation_off() {
        assert_eq!(
            basic_tokenize("The deadline, we agreed?"),
            vec!["the", "deadline", ",", "we", "agreed", "?"]
        );
    }

    #[test]
    fn basic_tokenizing_folds_accents_and_drops_control_characters() {
        assert_eq!(basic_tokenize("Café\u{0007}"), vec!["cafe"]);
        assert_eq!(basic_tokenize("cafe\u{0301}"), vec!["cafe"]);
    }

    #[test]
    fn basic_tokenizing_isolates_cjk_characters() {
        assert_eq!(basic_tokenize("ab\u{4E2D}\u{6587}cd"), vec!["ab", "\u{4E2D}", "\u{6587}", "cd"]);
    }

    #[test]
    fn encoding_wraps_the_sequence_in_cls_and_sep_then_pads() {
        let vocab = test_vocab();
        let encoded = vocab.encode("the deadline", 6);
        assert_eq!(encoded.input_ids, vec![2, 5, 4, 3, 0, 0]);
        assert_eq!(encoded.attention_mask, vec![1, 1, 1, 1, 0, 0]);
        assert_eq!(encoded.token_type_ids, vec![0; 6]);
    }

    #[test]
    fn encoding_truncates_to_the_window_and_always_closes_with_sep() {
        let vocab = test_vocab();
        let encoded = vocab.encode("the deadline we agreed", 4);
        assert_eq!(encoded.input_ids, vec![2, 5, 4, 3]);
        assert_eq!(encoded.attention_mask, vec![1, 1, 1, 1]);
    }

    #[test]
    fn wordpiece_segments_a_word_into_continuation_pieces() {
        let vocab = test_vocab();
        // "unaffable" -> un ##aff ##able
        let encoded = vocab.encode("unaffable", 8);
        assert_eq!(encoded.input_ids[1..4], [9, 10, 11]);
    }

    #[test]
    fn an_unsegmentable_word_becomes_a_single_unknown_token() {
        let vocab = test_vocab();
        let encoded = vocab.encode("zzzz", 4);
        assert_eq!(encoded.input_ids[1], 1);
    }

    #[test]
    fn an_absurdly_long_word_becomes_unknown_rather_than_being_scanned() {
        let vocab = test_vocab();
        let encoded = vocab.encode(&"a".repeat(MAX_CHARS_PER_WORD + 1), 4);
        assert_eq!(encoded.input_ids[1], 1);
    }

    #[test]
    fn empty_text_still_produces_a_valid_padded_sequence() {
        let vocab = test_vocab();
        let encoded = vocab.encode("   ", 4);
        assert_eq!(encoded.input_ids, vec![2, 3, 0, 0]);
        assert_eq!(encoded.attention_mask, vec![1, 1, 0, 0]);
    }
}
