//! "Write at cursor" (`@cursor`): with `"destinations": ["cursor", "session"]` on a dictation,
//! `scrawl router decide` may answer that the text belongs in the field in front rather than in
//! a session. Jev weighs it as one more target; an edit instruction over a selection in an
//! editable field settles it without Jev. The caller (Hex) writes it; `deliver` never sends it.

use super::jev::{self, Band, Record, Thresholds};
use super::smart::{Basis, Candidate, Decision, Judgement, Reason, Scored, Source};

/// The pseudo-session's ID and title.
pub const CURSOR: &str = "@cursor";
pub const CURSOR_TITLE: &str = "Write at cursor";
/// What Jev is told it means.
pub const CURSOR_ABOUT: &str = "Not a session: dictation meant for the text field in front, like \
rewriting a selection or typing a message.";

/// The end of an ask's reason, and its `why`, when nothing fits.
pub const NOTHING_FITS: &str = "nothing fits, so ask";

/// The longest selection passed on.
pub const SELECTION_CHARS: usize = 500;

/// How an edit instruction starts, after any [`FILLER`] (lowercase; a word boundary follows).
pub const EDIT_INSTRUCTIONS: &[&str] = &[
    "make this",
    "make it",
    "rewrite",
    "rephrase",
    "shorten",
    "shorter",
    "fix",
    "translate",
    "turn this into",
    "reword",
];

/// Words said before an instruction that don't change it (lowercase, longest first where one
/// starts another).
pub const FILLER: &[&str] = &[
    "okay",
    "ok",
    "umm",
    "um",
    "uhh",
    "uh",
    "hmm",
    "so",
    "hey",
    "can you",
    "could you",
    "would you",
    "will you",
    "please",
    "just",
];

pub fn candidate() -> Candidate {
    Candidate {
        id: CURSOR.into(),
        title: CURSOR_TITLE.into(),
        directory: None,
        repo: None,
        updated_ms: 0,
        viewed_ms: None,
    }
}

pub fn is_cursor(id: &str) -> bool {
    id == CURSOR
}

/// Whether `text` starts with an edit instruction ("Okay, can you rewrite this…").
pub fn is_edit_instruction(text: &str) -> bool {
    let lower = text.to_lowercase();
    let mut rest = lower.as_str();
    loop {
        rest = rest.trim_start_matches(|c: char| !c.is_alphanumeric());
        match FILLER.iter().find_map(|f| word_after(rest, f)) {
            Some(after) => rest = after,
            None => break,
        }
    }
    EDIT_INSTRUCTIONS
        .iter()
        .any(|i| word_after(rest, i).is_some())
}

/// What follows `prefix` at the start of `text` when a word ends there.
fn word_after<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let after = text.strip_prefix(prefix)?;
    after
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric())
        .then_some(after)
}

/// What the item tells about the field in front.
#[derive(Clone, Copy, Debug, Default)]
pub struct Field<'a> {
    pub editable: bool,
    pub selection: Option<&'a str>,
}

/// An edit instruction about a selection in an editable field goes to the cursor, unless a
/// session was named at the start (or chosen).
pub fn edit(text: &str, field: Field, smart: Judgement) -> Judgement {
    let named = match &smart.decision {
        Decision::Send { to, .. } => matches!(
            to.lead().map(|r| r.basis),
            Some(Basis::Chosen | Basis::Recorded | Basis::Clue(Source::Named))
        ),
        _ => false,
    };
    let selected = field.selection.is_some_and(|s| !s.trim().is_empty());
    if named || !field.editable || !selected || !is_edit_instruction(text) {
        return smart;
    }
    let instead = match &smart.decision {
        Decision::Send { to, .. } => vec![to.clone()],
        Decision::Offer { choices } => choices.iter().take(1).cloned().collect(),
        Decision::Keep { .. } => Vec::new(),
    };
    Judgement {
        decision: Decision::Send {
            to: scored(
                Basis::Clue(Source::CursorEdit),
                95,
                "an edit instruction about the selection in the field in front",
            ),
            instead,
        },
        ranking: smart.ranking,
    }
}

/// With nothing fitting (`any_fits` under the threshold), Jev's fallback to HOME becomes an
/// ask: the cursor first when the field in front is editable, else HOME then the cursor, then
/// Jev's pick.
pub fn when_nothing_fits(
    judgement: Judgement,
    mut record: Record,
    candidates: &[Candidate],
    editable: bool,
    thresholds: &Thresholds,
) -> (Judgement, Record) {
    let low = record.by == "jev"
        && matches!(record.band, Band::Fallback | Band::Keep)
        && record.fits.is_some_and(|f| f < thresholds.no_fit);
    if !low {
        return (judgement, record);
    }
    let p_of = |id: &str| {
        record
            .probabilities
            .iter()
            .find(|pick| pick.session == id)
            .map_or(0.0, |pick| pick.p)
    };
    let cursor = Scored {
        candidate: candidate(),
        ..scored(Basis::Jev, points(p_of(CURSOR)), "the field in front")
    };
    let home = jev::home_session(candidates).map(|home| Scored {
        candidate: home,
        ..scored(Basis::JevHome, 0, "HOME")
    });
    let mut choices: Vec<Scored> = if editable {
        std::iter::once(cursor).chain(home).collect()
    } else {
        home.into_iter().chain(std::iter::once(cursor)).collect()
    };
    if let Some(best) = record.target.as_ref().and_then(|pick| {
        candidates
            .iter()
            .find(|c| c.id == pick.session)
            .filter(|c| !choices.iter().any(|s| s.candidate.id == c.id))
            .map(|c| Scored {
                candidate: c.clone(),
                ..scored(Basis::Jev, points(pick.p), "Jev's pick")
            })
    }) {
        choices.push(best);
    }
    choices.truncate(jev::CHIPS);
    let decision = Decision::Offer { choices };
    record.band = Band::Ask;
    record.reason = format!("{}; {NOTHING_FITS}", record.reason);
    record.chips = jev::chips_of(&decision);
    (
        Judgement {
            decision,
            ranking: judgement.ranking,
        },
        record,
    )
}

fn points(p: f64) -> u32 {
    (p * 100.0).round() as u32
}

fn scored(basis: Basis, points: u32, why: &str) -> Scored {
    Scored {
        candidate: candidate(),
        score: points,
        reasons: vec![Reason {
            basis,
            points,
            why: why.into(),
            directory: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_instructions_are_found_after_filler() {
        for text in [
            "Make this shorter",
            "make it more formal",
            "Okay, um, can you rewrite this as a list",
            "please rephrase that",
            "Uh, so, shorten it",
            "fix the typos",
            "TRANSLATE to French",
            "turn this into bullet points",
            "reword.",
            "Could you just make this friendlier?",
        ] {
            assert!(is_edit_instruction(text), "{text}");
        }
        for text in [
            "the login button is broken, fix it",
            "fixing the build is next",
            "makeup of the team",
            "tell scrawl to rewrite the docs later",
            "okay",
            "",
        ] {
            assert!(!is_edit_instruction(text), "{text}");
        }
    }
}
