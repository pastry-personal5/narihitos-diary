//! The M7 Command Line: a bounded application parser (`help`, `retry`,
//! `day YYYY-MM-DD`, `stage <canonical-stage>`, `save`, `quit`), not a
//! shell.
//!
//! Real-terminal observation (2026-10-29, see AGENTS.md "Keybindings")
//! found that Apple Terminal's 2-Set Korean IME delivers a **mix**: a bare
//! Hangul Compatibility Jamo (U+3131–U+3163) when a key does not complete a
//! syllable, and a fully committed precomposed syllable (U+AC00–U+D7A3,
//! e.g. 디) when a later key completes one. So normalization must handle
//! both: a bare jamo maps to its one 2-Set key directly, and a precomposed
//! syllable is decomposed (via the standard Unicode Hangul algorithm) into
//! its lead/vowel/optional-tail jamo, each then mapped to the 2-Set key(s)
//! that produce it — a compound vowel or tail consonant cluster expands to
//! two keys, since 2-Set types those as two separate keystrokes.

use crate::stage::Stage;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Help,
    Retry,
    Day(String),
    Stage(Stage),
    Save,
    Quit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandLineError {
    UnmappableCharacter,
    UnknownToken,
    MalformedArgument,
}

const SYLLABLE_BASE: u32 = 0xAC00;
const SYLLABLE_END: u32 = 0xD7A3;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;

/// 2-Set leading-consonant (choseong) keys, indexed 0–18 in Unicode order.
/// Doubled consonants (ㄲㄸㅃㅆㅉ) collapse to their base key: this
/// bounded grammar never needs the shift distinction.
const LEAD_KEYS: [char; 19] = [
    'r', 'r', 's', 'e', 'e', 'f', 'a', 'q', 'q', 't', 't', 'd', 'w', 'w', 'c', 'z', 'x', 'v', 'g',
];

/// 2-Set vowel (jungseong) key sequences, indexed 0–20 in Unicode order. A
/// compound vowel is two keystrokes on 2-Set.
const VOWEL_KEYS: [&[char]; 21] = [
    &['k'],      // ㅏ
    &['o'],      // ㅐ
    &['i'],      // ㅑ
    &['o'],      // ㅒ (shift+ㅐ, collapses to the same key)
    &['j'],      // ㅓ
    &['p'],      // ㅔ
    &['u'],      // ㅕ
    &['p'],      // ㅖ (shift+ㅔ, collapses to the same key)
    &['h'],      // ㅗ
    &['h', 'k'], // ㅘ = ㅗ+ㅏ
    &['h', 'o'], // ㅙ = ㅗ+ㅐ
    &['h', 'l'], // ㅚ = ㅗ+ㅣ
    &['y'],      // ㅛ
    &['n'],      // ㅜ
    &['n', 'j'], // ㅝ = ㅜ+ㅓ
    &['n', 'p'], // ㅞ = ㅜ+ㅔ
    &['n', 'l'], // ㅟ = ㅜ+ㅣ
    &['b'],      // ㅠ
    &['m'],      // ㅡ
    &['m', 'l'], // ㅢ = ㅡ+ㅣ
    &['l'],      // ㅣ
];

/// 2-Set trailing-consonant (jongseong) key sequences, indexed 0–27 in
/// Unicode order; index 0 means "no trailing consonant".
const TAIL_KEYS: [&[char]; 28] = [
    &[],         // (none)
    &['r'],      // ㄱ
    &['r'],      // ㄲ (collapses to the base key)
    &['r', 't'], // ㄳ = ㄱ+ㅅ
    &['s'],      // ㄴ
    &['s', 'w'], // ㄵ = ㄴ+ㅈ
    &['s', 'g'], // ㄶ = ㄴ+ㅎ
    &['e'],      // ㄷ
    &['f'],      // ㄹ
    &['f', 'r'], // ㄺ = ㄹ+ㄱ
    &['f', 'a'], // ㄻ = ㄹ+ㅁ
    &['f', 'q'], // ㄼ = ㄹ+ㅂ
    &['f', 't'], // ㄽ = ㄹ+ㅅ
    &['f', 'x'], // ㄾ = ㄹ+ㅌ
    &['f', 'v'], // ㄿ = ㄹ+ㅍ
    &['f', 'g'], // ㅀ = ㄹ+ㅎ
    &['a'],      // ㅁ
    &['q'],      // ㅂ
    &['q', 't'], // ㅄ = ㅂ+ㅅ
    &['t'],      // ㅅ
    &['t'],      // ㅆ (collapses to the base key)
    &['d'],      // ㅇ
    &['w'],      // ㅈ
    &['c'],      // ㅊ
    &['z'],      // ㅋ
    &['x'],      // ㅌ
    &['v'],      // ㅍ
    &['g'],      // ㅎ
];

/// 2-Set key sequences for the Hangul Compatibility Jamo consonant range
/// (U+3131–U+314E), in exact codepoint order. This is **not** the same
/// order as `TAIL_KEYS`: the Compatibility Jamo block also includes the
/// doubled consonants ㄸ, ㅃ, and ㅉ, which can never be a syllable's
/// final consonant and so are absent from the jongseong-only `TAIL_KEYS`
/// order — reusing `TAIL_KEYS` here shifted every later entry and was a
/// real bug caught by `jamo_vector`'s roundtrip test (`retry` normalized
/// to `redry`).
const COMPAT_CONSONANT_KEYS: [&[char]; 30] = [
    &['r'],      // ㄱ
    &['r'],      // ㄲ (collapses to the base key)
    &['r', 't'], // ㄳ = ㄱ+ㅅ
    &['s'],      // ㄴ
    &['s', 'w'], // ㄵ = ㄴ+ㅈ
    &['s', 'g'], // ㄶ = ㄴ+ㅎ
    &['e'],      // ㄷ
    &['e'],      // ㄸ (collapses to the base key)
    &['f'],      // ㄹ
    &['f', 'r'], // ㄺ = ㄹ+ㄱ
    &['f', 'a'], // ㄻ = ㄹ+ㅁ
    &['f', 'q'], // ㄼ = ㄹ+ㅂ
    &['f', 't'], // ㄽ = ㄹ+ㅅ
    &['f', 'x'], // ㄾ = ㄹ+ㅌ
    &['f', 'v'], // ㄿ = ㄹ+ㅍ
    &['f', 'g'], // ㅀ = ㄹ+ㅎ
    &['a'],      // ㅁ
    &['q'],      // ㅂ
    &['q'],      // ㅃ (collapses to the base key)
    &['q', 't'], // ㅄ = ㅂ+ㅅ
    &['t'],      // ㅅ
    &['t'],      // ㅆ (collapses to the base key)
    &['d'],      // ㅇ
    &['w'],      // ㅈ
    &['w'],      // ㅉ (collapses to the base key)
    &['c'],      // ㅊ
    &['z'],      // ㅋ
    &['x'],      // ㅌ
    &['v'],      // ㅍ
    &['g'],      // ㅎ
];

const COMPAT_CONSONANT_START: u32 = 0x3131; // ㄱ
const COMPAT_CONSONANT_END: u32 = 0x314E; // ㅎ
const COMPAT_VOWEL_START: u32 = 0x314F; // ㅏ
const COMPAT_VOWEL_END: u32 = 0x3163; // ㅣ

/// Maps a single bare (non-syllable) incoming character to its 2-Set key
/// sequence: ASCII passes through unchanged (lowercased) as one key, and
/// any Hangul Compatibility Jamo (U+3131–U+3163) — including compound
/// vowels and consonant clusters, which a 2026-10-29 real-terminal
/// observation found can also arrive bare, not only inside a composed
/// syllable — maps to the one or two keys that produce it. Vowels reuse
/// `VOWEL_KEYS` (the Compatibility Jamo vowel order matches the syllable
/// jungseong order exactly); consonants use the dedicated
/// `COMPAT_CONSONANT_KEYS` table, not `TAIL_KEYS` (see its doc comment).
fn bare_jamo_keys(ch: char) -> Option<&'static [char]> {
    let code = u32::from(ch);
    if (COMPAT_CONSONANT_START..=COMPAT_CONSONANT_END).contains(&code) {
        let index = (code - COMPAT_CONSONANT_START) as usize;
        return Some(COMPAT_CONSONANT_KEYS[index]);
    }
    if (COMPAT_VOWEL_START..=COMPAT_VOWEL_END).contains(&code) {
        let index = (code - COMPAT_VOWEL_START) as usize;
        return Some(VOWEL_KEYS[index]);
    }
    None
}

/// Converts raw Command Line input (which may mix ASCII, bare 2-Set
/// Hangul jamo — simple or compound — and fully committed precomposed
/// syllables) into the lowercase ASCII the grammar below matches against.
pub fn normalize(raw: &str) -> Result<String, CommandLineError> {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if let Some(keys) = syllable_keys(ch) {
            out.extend(keys);
            continue;
        }
        if let Some(keys) = bare_jamo_keys(ch) {
            out.extend(keys);
            continue;
        }
        if ch.is_ascii() {
            out.push(ch.to_ascii_lowercase());
            continue;
        }
        return Err(CommandLineError::UnmappableCharacter);
    }
    Ok(out)
}

/// Decomposes one precomposed Hangul syllable into the 2-Set key sequence
/// that would type it, in order (lead, then vowel keys, then tail keys).
fn syllable_keys(ch: char) -> Option<Vec<char>> {
    let code = u32::from(ch);
    if !(SYLLABLE_BASE..=SYLLABLE_END).contains(&code) {
        return None;
    }
    let index = code - SYLLABLE_BASE;
    let lead = (index / (V_COUNT * T_COUNT)) as usize;
    let vowel = ((index / T_COUNT) % V_COUNT) as usize;
    let tail = (index % T_COUNT) as usize;
    let mut keys = Vec::with_capacity(4);
    keys.push(LEAD_KEYS[lead]);
    keys.extend_from_slice(VOWEL_KEYS[vowel]);
    keys.extend_from_slice(TAIL_KEYS[tail]);
    Some(keys)
}

/// Parses already-normalized (lowercase ASCII) Command Line input against
/// the bounded M7 grammar.
pub fn parse(normalized: &str) -> Result<Command, CommandLineError> {
    let mut parts = normalized.split_whitespace();
    let token = parts.next().ok_or(CommandLineError::UnknownToken)?;
    let command = match token {
        "help" => Command::Help,
        "retry" => Command::Retry,
        "save" => Command::Save,
        "quit" => Command::Quit,
        "day" => {
            let argument = parts.next().ok_or(CommandLineError::MalformedArgument)?;
            if !is_valid_day(argument) {
                return Err(CommandLineError::MalformedArgument);
            }
            Command::Day(argument.to_owned())
        }
        "stage" => {
            let argument = parts.next().ok_or(CommandLineError::MalformedArgument)?;
            let stage = match argument {
                "start" => Stage::Start,
                "sources" => Stage::Sources,
                "compose" => Stage::Compose,
                "review" => Stage::Review,
                "preview" => Stage::Preview,
                "publish" | "publish-local" | "local" => Stage::PublishLocal,
                "publish-remote" | "remote" => Stage::PublishRemote,
                _ => return Err(CommandLineError::MalformedArgument),
            };
            Command::Stage(stage)
        }
        _ => return Err(CommandLineError::UnknownToken),
    };
    if parts.next().is_some() {
        return Err(CommandLineError::MalformedArgument);
    }
    Ok(command)
}

/// The Command Line's raw input buffer. Holds un-normalized characters as
/// typed (ASCII or Korean jamo/syllables); normalization and parsing both
/// happen together at `submit`, not per keystroke, since a precomposed
/// syllable can expand into multiple ASCII characters.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommandLineState {
    raw: String,
}

pub const MAX_COMMAND_BYTES: usize = 256;

impl CommandLineState {
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn push_char(&mut self, ch: char) {
        if self.raw.len().saturating_add(ch.len_utf8()) <= MAX_COMMAND_BYTES {
            self.raw.push(ch);
        }
    }

    /// Removes the last raw character. Note this removes one `char`
    /// (Unicode scalar value), which for a precomposed syllable removes
    /// the whole syllable in one step, matching how it was typed.
    pub fn backspace(&mut self) {
        self.raw.pop();
    }

    pub fn clear(&mut self) {
        self.raw.clear();
    }

    /// Normalizes and parses the buffered input, then clears it
    /// regardless of outcome — a failed command is not retried silently
    /// and does not linger half-typed.
    pub fn submit(&mut self) -> Result<Command, CommandLineError> {
        let raw = std::mem::take(&mut self.raw);
        let normalized = normalize(&raw)?;
        parse(&normalized)
    }
}

pub fn is_valid_day(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .bytes()
            .enumerate()
            .all(|(index, byte)| index == 4 || index == 7 || byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_characters_submit_as_a_parsed_command_and_clear_the_buffer() {
        let mut state = CommandLineState::default();
        for ch in "quit".chars() {
            state.push_char(ch);
        }
        assert_eq!(state.submit(), Ok(Command::Quit));
        assert_eq!(state.raw(), "");
    }

    #[test]
    fn raw_input_is_bounded_without_splitting_utf8() {
        let mut state = CommandLineState::default();
        for _ in 0..MAX_COMMAND_BYTES {
            state.push_char('한');
        }
        assert!(state.raw().len() <= MAX_COMMAND_BYTES);
        assert!(state.raw().is_char_boundary(state.raw().len()));
    }

    #[test]
    fn backspace_removes_the_last_character() {
        let mut state = CommandLineState::default();
        state.push_char('s');
        state.push_char('x');
        state.backspace();
        state.push_char('v');
        assert_eq!(state.raw(), "sv");
    }

    #[test]
    fn backspace_removes_a_whole_precomposed_syllable_as_one_step() {
        let mut state = CommandLineState::default();
        state.push_char('h');
        state.push_char('\u{B514}'); // 디, as one committed syllable
        state.backspace();
        assert_eq!(state.raw(), "h");
    }

    #[test]
    fn submit_clears_the_buffer_even_when_parsing_fails() {
        let mut state = CommandLineState::default();
        state.push_char('x');
        state.push_char('x');
        assert_eq!(state.submit(), Err(CommandLineError::UnknownToken));
        assert_eq!(state.raw(), "");
    }

    /// 2026-10-29 real-terminal observation: typing produced the
    /// precomposed syllable 디 (U+B514) committed as one block, mixed in
    /// with bare jamo in the same line. 디 = ㄷ(lead)+ㅣ(vowel), which on
    /// 2-Set is the keys `e` then `l`.
    #[test]
    fn the_observed_precomposed_syllable_decomposes_to_its_2_set_keys() {
        assert_eq!(normalize("\u{B514}").unwrap(), "el");
    }

    #[test]
    fn a_syllable_with_a_simple_tail_consonant_decomposes_to_three_keys() {
        // 각 = ㄱ+ㅏ+ㄱ -> r, k, r
        assert_eq!(normalize("\u{AC01}").unwrap(), "rkr");
    }

    #[test]
    fn a_syllable_with_a_compound_vowel_and_tail_cluster_decomposes_fully() {
        // 왔 = ㅇ+ㅘ(ㅗ+ㅏ)+ㅆ(collapsed to ㅅ) -> d, h, k, t
        assert_eq!(normalize("\u{C654}").unwrap(), "dhkt");
    }

    /// 2026-10-29 real-terminal observation: a bare compound vowel jamo
    /// (ㅘ, U+3158) arrived standalone, not inside a composed syllable,
    /// and was incorrectly rejected as unmappable before this fix.
    #[test]
    fn a_bare_compound_vowel_jamo_maps_to_its_two_keys() {
        assert_eq!(normalize("\u{3158}").unwrap(), "hk"); // ㅘ = ㅗ+ㅏ
    }

    #[test]
    fn a_bare_consonant_cluster_jamo_maps_to_its_two_keys() {
        assert_eq!(normalize("\u{3133}").unwrap(), "rt"); // ㄳ = ㄱ+ㅅ
    }

    #[test]
    fn a_syllable_with_no_tail_consonant_has_no_trailing_keys() {
        // 가 = ㄱ+ㅏ, no tail -> r, k
        assert_eq!(normalize("\u{AC00}").unwrap(), "rk");
    }

    #[test]
    fn a_mix_of_bare_jamo_and_precomposed_syllables_normalizes_together() {
        // The exact shape observed live: a bare vowel jamo, a precomposed
        // syllable, then another bare vowel jamo in the same input.
        assert_eq!(
            normalize("\u{3157}\u{B514}\u{3154}").unwrap(),
            "h".to_owned() + "el" + "p"
        );
    }

    /// For each token, the exact 2-Set jamo sequence a Korean-IME user
    /// would produce by pressing the token's Latin keys, per the real
    /// terminal observation: raw per-key jamo, not a composed syllable.
    fn jamo_vector(ascii: &str) -> String {
        ascii
            .chars()
            .map(|c| match c {
                'h' => '\u{3157}',
                'e' => '\u{3137}',
                'l' => '\u{3163}',
                'p' => '\u{3154}',
                'r' => '\u{3131}',
                't' => '\u{3145}',
                'y' => '\u{315B}',
                'd' => '\u{3147}',
                'a' => '\u{3141}',
                's' => '\u{3134}',
                'g' => '\u{314E}',
                'v' => '\u{314D}',
                'q' => '\u{3142}',
                'u' => '\u{3155}',
                'i' => '\u{3151}',
                other => other,
            })
            .collect()
    }

    #[test]
    fn ascii_tokens_parse_directly() {
        assert_eq!(parse(&normalize("help").unwrap()).unwrap(), Command::Help);
        assert_eq!(parse(&normalize("retry").unwrap()).unwrap(), Command::Retry);
        assert_eq!(parse(&normalize("save").unwrap()).unwrap(), Command::Save);
        assert_eq!(parse(&normalize("quit").unwrap()).unwrap(), Command::Quit);
        assert_eq!(
            parse(&normalize("day 2026-10-29").unwrap()).unwrap(),
            Command::Day("2026-10-29".to_owned())
        );
        assert_eq!(
            parse(&normalize("stage sources").unwrap()).unwrap(),
            Command::Stage(Stage::Sources)
        );
    }

    #[test]
    fn a_decomposed_jamo_vector_exists_for_every_grammar_token() {
        for (token, expected) in [
            ("help", Command::Help),
            ("retry", Command::Retry),
            ("save", Command::Save),
            ("quit", Command::Quit),
        ] {
            let jamo = jamo_vector(token);
            assert_ne!(jamo, token, "{token} vector must actually use jamo");
            let normalized = normalize(&jamo).unwrap();
            assert_eq!(normalized, token);
            assert_eq!(parse(&normalized).unwrap(), expected);
        }

        let day_jamo = format!("{} 2026-10-29", jamo_vector("day"));
        assert_eq!(normalize(&day_jamo).unwrap(), "day 2026-10-29");
        assert_eq!(
            parse(&normalize(&day_jamo).unwrap()).unwrap(),
            Command::Day("2026-10-29".to_owned())
        );

        let stage_jamo = format!("{} sources", jamo_vector("stage"));
        assert_eq!(normalize(&stage_jamo).unwrap(), "stage sources");
        assert_eq!(
            parse(&normalize(&stage_jamo).unwrap()).unwrap(),
            Command::Stage(Stage::Sources)
        );
    }

    #[test]
    fn an_unmappable_non_ascii_character_is_rejected() {
        assert_eq!(
            normalize("\u{65E5}").unwrap_err(),
            CommandLineError::UnmappableCharacter
        );
    }

    #[test]
    fn an_unknown_token_is_rejected() {
        assert_eq!(
            parse(&normalize("rm -rf").unwrap()).unwrap_err(),
            CommandLineError::UnknownToken
        );
    }

    #[test]
    fn an_extra_argument_is_rejected() {
        assert_eq!(
            parse(&normalize("save now").unwrap()).unwrap_err(),
            CommandLineError::MalformedArgument
        );
    }

    #[test]
    fn a_malformed_day_argument_is_rejected() {
        assert_eq!(
            parse(&normalize("day not-a-date").unwrap()).unwrap_err(),
            CommandLineError::MalformedArgument
        );
    }
}
