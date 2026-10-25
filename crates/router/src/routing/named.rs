//! A session named at the start of what was said ("tell scrawl …", "for rune: …", "scrawl, …",
//! "hey hex …"): an address, obeyed ahead of Jev (`docs/routing.md`).
//!
//! Only an addressing phrase at the very start counts, never a name elsewhere in the text, so
//! "scroll down" isn't an address even when `scroll` is an alias.

use std::path::Path;

use super::smart::Candidate;

/// The session the text opens by addressing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Named {
    pub id: String,
    /// The words that named it, as said.
    pub spoken: String,
    /// `title`, `folder` or `alias`, and how closely: `exact`, `prefix` or `one edit`.
    pub how: String,
}

/// Words dropped wherever they fall in the opening.
const FILLER: &[&str] = &[
    "um", "uh", "uhh", "umm", "erm", "er", "hmm", "mm", "okay", "ok", "so", "alright", "well",
    "oh", "and", "please", "actually",
];
/// Greetings: the name follows, with or without punctuation.
const GREETINGS: &[&str] = &["hey", "hi", "hello", "yo"];
/// Verbs whose object is the session: the name follows, with or without punctuation.
const VERBS: &[&str] = &["tell", "ask", "ping"];
/// Prepositions: the name follows and ends at a `:` or `,`.
const PREPOSITIONS: &[&str] = &["for", "to"];
/// Words that may follow the name ("the scrawl session").
const NOUNS: &[&str] = &["session", "agent", "tab", "chat", "thread"];
/// The longest name, in words.
const NAME_WORDS: usize = 3;
/// How many words of the opening are read.
const OPENING: usize = 12;

#[derive(Debug)]
struct Token {
    word: String,
    /// Followed by `,`, `:`, `;` or a dash.
    stops: bool,
}

/// The session `text` opens by addressing, among `candidates`. `aliases` map spoken words to a
/// title or folder name (`routing.aliases`, e.g. `{"scroll": "SCRAWL"}`).
pub fn addressed(
    text: &str,
    candidates: &[Candidate],
    aliases: &[(String, String)],
) -> Option<Named> {
    let tokens = opening(text);
    let first = tokens.first()?;
    let spans: Vec<&[Token]> =
        if GREETINGS.contains(&first.word.as_str()) || VERBS.contains(&first.word.as_str()) {
            let rest = skip_the(&tokens[1..]);
            let end = stop(rest).map_or(rest.len(), |i| i + 1);
            (1..=NAME_WORDS.min(end))
                .rev()
                .map(|n| &rest[..n])
                .collect()
        } else if PREPOSITIONS.contains(&first.word.as_str()) {
            let rest = skip_the(&tokens[1..]);
            punctuated(rest).into_iter().collect()
        } else {
            punctuated(&tokens).into_iter().collect()
        };
    // Every span competes on how well it matches, the longer winning a tie, so "quil to" one
    // edit from one title can't beat "quil" naming two.
    spans
        .into_iter()
        .filter_map(|span| resolve(span, candidates, aliases).map(|m| (span.len(), m)))
        .min_by(|(a_len, a), (b_len, b)| a.0.cmp(&b.0).then(b_len.cmp(a_len)))
        .and_then(|(_, (_, named))| named)
}

/// The first words of `text`, lowercased, without filler.
fn opening(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    for raw in text.split_whitespace() {
        if tokens.len() >= OPENING {
            break;
        }
        let word: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        let stops = raw
            .trim_end_matches(|c: char| c == '"' || c == '\u{201d}' || c == '\'')
            .ends_with([',', ':', ';', '\u{2014}', '\u{2013}']);
        if word.is_empty() {
            // A lone dash or colon ends the word before it.
            if stops && let Some(last) = tokens.last_mut() {
                let last: &mut Token = last;
                last.stops = true;
            }
            continue;
        }
        if FILLER.contains(&word.as_str()) {
            continue;
        }
        tokens.push(Token { word, stops });
    }
    tokens
}

fn skip_the(tokens: &[Token]) -> &[Token] {
    match tokens.first() {
        Some(t) if t.word == "the" && !t.stops => &tokens[1..],
        _ => tokens,
    }
}

/// The index of the first word followed by punctuation.
fn stop(tokens: &[Token]) -> Option<usize> {
    tokens.iter().position(|t| t.stops)
}

/// The words up to a `:` or `,` within [`NAME_WORDS`] of the start.
fn punctuated(tokens: &[Token]) -> Option<&[Token]> {
    let end = stop(tokens)?;
    (end < NAME_WORDS).then(|| &tokens[..=end])
}

/// How a spoken name matches a session's, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Title,
    Folder,
    TitlePrefix,
    FolderPrefix,
    TitleEdit,
    FolderEdit,
}

impl Level {
    fn how(self) -> &'static str {
        match self {
            Self::Title => "title, exact",
            Self::Folder => "folder, exact",
            Self::TitlePrefix => "title, prefix",
            Self::FolderPrefix => "folder, prefix",
            Self::TitleEdit => "title, one edit",
            Self::FolderEdit => "folder, one edit",
        }
    }
}

/// The best match for `span`, `None` inside when two different names match as well.
fn resolve(
    span: &[Token],
    candidates: &[Candidate],
    aliases: &[(String, String)],
) -> Option<(Level, Option<Named>)> {
    let mut words: Vec<&str> = span.iter().map(|t| t.word.as_str()).collect();
    if words.len() > 1 && words.last().is_some_and(|w| NOUNS.contains(w)) {
        words.pop();
    }
    let spoken = words.join(" ");
    let said = words.concat();
    let alias = aliases
        .iter()
        .find(|(from, _)| squash(from) == said)
        .map(|(_, to)| squash(to));
    let key = alias.clone().unwrap_or(said);
    // "A, the button…" names no session called A.
    if key.chars().count() < 2 {
        return None;
    }
    let mut best: Option<(Level, Vec<(&Candidate, String)>)> = None;
    for candidate in candidates {
        let Some((level, name)) = level_of(&key, candidate, alias.is_some()) else {
            continue;
        };
        match &mut best {
            Some((at, found)) if *at == level => found.push((candidate, name)),
            Some((at, _)) if *at < level => {}
            _ => best = Some((level, vec![(candidate, name)])),
        }
    }
    let (level, found) = best?;
    // Two different names matching as well as each other is no address.
    if found.iter().any(|(_, name)| *name != found[0].1) {
        return Some((level, None));
    }
    let candidate = found
        .iter()
        .map(|(c, _)| *c)
        .max_by(|a, b| a.updated_ms.cmp(&b.updated_ms).then(b.id.cmp(&a.id)))?;
    let named = Named {
        id: candidate.id.clone(),
        spoken,
        how: if alias.is_some() {
            format!("alias, {}", level.how())
        } else {
            level.how().to_owned()
        },
    };
    Some((level, Some(named)))
}

/// How `key` names `candidate`, and the name it matched. An alias must match exactly.
fn level_of(key: &str, candidate: &Candidate, exact_only: bool) -> Option<(Level, String)> {
    let title = squash(&candidate.title);
    let folders = folder_names(candidate);
    let mut levels = vec![(Level::Title, title.clone())];
    levels.extend(folders.iter().map(|f| (Level::Folder, f.clone())));
    if !exact_only {
        levels.push((Level::TitlePrefix, title.clone()));
        levels.extend(folders.iter().map(|f| (Level::FolderPrefix, f.clone())));
        levels.push((Level::TitleEdit, title));
        levels.extend(folders.into_iter().map(|f| (Level::FolderEdit, f)));
    }
    levels.into_iter().find(|(level, name)| {
        !name.is_empty()
            && match level {
                Level::Title | Level::Folder => name == key,
                Level::TitlePrefix | Level::FolderPrefix => {
                    key.chars().count() >= 3 && name.starts_with(key)
                }
                Level::TitleEdit | Level::FolderEdit => {
                    key.chars().count() >= 4 && one_edit(key, name)
                }
            }
    })
}

/// The names of its folder and repository, skipping OpenCode's made-up worktree names.
fn folder_names(candidate: &Candidate) -> Vec<String> {
    let mut names = Vec::new();
    for path in [candidate.directory.as_deref(), candidate.repo.as_deref()]
        .into_iter()
        .flatten()
    {
        if path.components().any(|c| c.as_os_str() == "worktree") || is_home(path) {
            continue;
        }
        if let Some(name) = path.file_name().map(|n| squash(&n.to_string_lossy()))
            && !name.is_empty()
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

fn is_home(path: &Path) -> bool {
    super::smart::is_home_or_root(path)
}

/// Lowercase letters and digits only: "voice-control" and "Voice Control" are one name.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Whether `a` and `b` differ by at most one inserted, deleted or changed character.
fn one_edit(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let (short, long) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    if long.len() - short.len() > 1 {
        return false;
    }
    let lead = short.iter().zip(&long).take_while(|(x, y)| x == y).count();
    if short.len() == long.len() {
        short[lead..].iter().skip(1).eq(long[lead..].iter().skip(1))
    } else {
        short[lead..].iter().eq(long[lead + 1..].iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn session(id: &str, title: &str, dir: &str, updated_ms: u64) -> Candidate {
        Candidate {
            id: id.into(),
            title: title.into(),
            directory: Some(PathBuf::from(dir)),
            repo: None,
            updated_ms,
            viewed_ms: None,
        }
    }

    fn sessions() -> Vec<Candidate> {
        vec![
            session("ses_quill", "QUILL", "/u/code/quill", 5),
            session("ses_lark", "LARK", "/u/code/lark-studio", 4),
            session("ses_ember", "Ember Tools", "/u/code/ember", 3),
            session("ses_home", "HOME", "/u", 2),
            session("ses_quill_old", "QUILL", "/u/code/quill", 1),
        ]
    }

    fn to(text: &str) -> Option<String> {
        to_with(text, &[])
    }

    fn to_with(text: &str, aliases: &[(&str, &str)]) -> Option<String> {
        let aliases: Vec<(String, String)> = aliases
            .iter()
            .map(|(a, b)| ((*a).into(), (*b).into()))
            .collect();
        addressed(text, &sessions(), &aliases).map(|n| n.id)
    }

    #[test]
    fn addressing_phrases_at_the_start_name_a_session() {
        let quill = Some("ses_quill".to_owned());
        assert_eq!(to("tell quill to fix the margins"), quill);
        assert_eq!(to("Tell the Quill session the build is green"), quill);
        assert_eq!(to("for quill: rename the flag"), quill);
        assert_eq!(to("Quill, why is this slow?"), quill);
        assert_eq!(to("Hey Quill can you look"), quill);
        assert_eq!(to("Okay, um, hey, Quill, one more thing"), quill);
        assert_eq!(to("ask lark whether it shipped"), Some("ses_lark".into()));
        assert_eq!(to("Ember tools: the build broke"), Some("ses_ember".into()));
        assert_eq!(to("tell home to file it"), Some("ses_home".into()));
    }

    #[test]
    fn a_name_anywhere_else_is_not_an_address() {
        assert_eq!(to("fix quill's margins"), None);
        assert_eq!(to("the build for quill broke"), None);
        assert_eq!(to("quill the margins are off"), None);
        assert_eq!(to("for the record, quill is slow"), None);
        assert_eq!(to("tell me what quill does"), None);
        assert_eq!(to(""), None);
        let one_letter = [session("ses_a", "A", "/u/code/a", 9)];
        assert_eq!(addressed("A, the button", &one_letter, &[]), None);
    }

    #[test]
    fn titles_then_prefixes_then_one_edit_then_folders() {
        // A prefix of a title, a folder's name, one edit away.
        assert_eq!(to("tell qui to look"), Some("ses_quill".into()));
        assert_eq!(to("tell lark studio to look"), Some("ses_lark".into()));
        assert_eq!(to("tell qill to look"), Some("ses_quill".into()));
        assert_eq!(to("tell embr to look"), Some("ses_ember".into()));
        // Short words don't get a prefix or an edit.
        assert_eq!(to("hey qu, look"), None);
        assert_eq!(to("tell lak to look"), None);
    }

    #[test]
    fn two_sessions_named_alike_go_to_the_newer_and_two_names_to_none() {
        assert_eq!(to("quill, look"), Some("ses_quill".into()));
        let mut both = sessions();
        both.push(session("ses_quilt", "QUILT", "/u/code/quilt", 9));
        assert_eq!(
            addressed("tell quil to look", &both, &[]).map(|n| n.id),
            None
        );
        assert_eq!(
            addressed("tell quill to look", &both, &[]).map(|n| n.id),
            Some("ses_quill".into())
        );
    }

    #[test]
    fn aliases_name_a_title_or_folder_exactly() {
        let aliases = [("scroll", "QUILL"), ("larks", "lark-studio")];
        assert_eq!(
            to_with("tell scroll to look", &aliases),
            Some("ses_quill".into())
        );
        assert_eq!(to_with("Larks: look", &aliases), Some("ses_lark".into()));
        // An alias is still only an address in an addressing phrase.
        assert_eq!(to_with("scroll down to the footer", &aliases), None);
        assert_eq!(to_with("scroll down, please", &aliases), None);
    }

    #[test]
    fn the_match_says_how() {
        let named = addressed("hey qill, look", &sessions(), &[]).unwrap();
        assert_eq!(named.spoken, "qill");
        assert_eq!(named.how, "title, one edit");
        let named = addressed("tell the quill session to look", &sessions(), &[]).unwrap();
        assert_eq!(named.spoken, "quill");
        assert_eq!(named.how, "title, exact");
    }
}
