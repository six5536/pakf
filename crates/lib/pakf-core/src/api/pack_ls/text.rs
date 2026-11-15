//! This module renders the text of `pakf ls`: the short form,
//! the columns of `-l`, and the colours.

use std::fmt::Write as _;

use super::{PackAcceptance, PackFileKind, PackLs, PackLsEntry, PackLsState};

/// `ACCEPTED` is the SGR parameter of the colour of an acceptance.
const ACCEPTED: &str = "36";

impl PackLs {
    /// `to_text` returns a line for each entry, under a line that names its
    /// directory when the report has headers, with a blank line between the
    /// groups. With `long`, each line holds the entry's state, acceptance,
    /// pack, and kind in columns before its name, then the note of a file's
    /// acceptance or the numbers of a directory's files of packs. With
    /// `colour`, the name, the state, and the acceptance of each entry that
    /// has a state take the entry's colour.
    // sokf-implements: CLI-5_AC-14
    // sokf-implements: CLI-5_AC-27
    pub fn to_text(&self, long: bool, colour: bool) -> String {
        let mut out = String::new();
        for (i, listing) in self.listings.iter().enumerate() {
            if i > 0 {
                out.push('\n');
            }
            if let Some(dir) = listing.dir.as_ref().filter(|_| self.headers) {
                let _ = writeln!(out, "{dir}:");
            }
            let names: Vec<String> = listing
                .entries
                .iter()
                .map(|e| name(e, listing.dir.is_some()))
                .collect();
            if !long {
                for (e, n) in listing.entries.iter().zip(&names) {
                    let _ = writeln!(out, "{}", paint(n, e, colour));
                }
                continue;
            }
            let cells: Vec<[&str; 4]> = listing
                .entries
                .iter()
                .map(|e| {
                    [
                        e.state.map_or("-", PackLsState::label),
                        e.acceptance.map_or("-", PackAcceptance::label),
                        e.pack.as_deref().unwrap_or("-"),
                        e.kind.map_or("-", PackFileKind::label),
                    ]
                })
                .collect();
            let width = |c: usize| cells.iter().map(|r| r[c].len()).max().unwrap_or(0);
            let widths = [width(0), width(1), width(2), width(3)];
            for ((e, n), row) in listing.entries.iter().zip(&names).zip(&cells) {
                let pad = |c: usize| " ".repeat(widths[c] - row[c].len());
                let acceptance = match e.acceptance {
                    Some(_) => paint(row[1], e, colour),
                    None => row[1].to_string(),
                };
                let _ = write!(
                    out,
                    "{}{}  {acceptance}{}  {}{}  {}{}  {}",
                    paint(row[0], e, colour),
                    pad(0),
                    pad(1),
                    row[2],
                    pad(2),
                    row[3],
                    pad(3),
                    paint(n, e, colour),
                );
                if let Some(note) = &e.note {
                    let _ = write!(out, ": {note}");
                }
                if let Some(counts) = e.counts.filter(|c| c.files > 0) {
                    let states: Vec<String> = PackLsState::ALL
                        .iter()
                        .filter(|s| counts.of(**s) > 0)
                        .map(|s| format!("{} {}", counts.of(*s), s.label()))
                        .collect();
                    let acceptances: Vec<String> = PackAcceptance::ALL
                        .iter()
                        .filter(|a| counts.with(**a) > 0)
                        .map(|a| format!("{} {}", counts.with(*a), a.label()))
                        .collect();
                    let files = if counts.files == 1 { "file" } else { "files" };
                    let _ = write!(
                        out,
                        " ({} pack {files}: {}",
                        counts.files,
                        states.join(", ")
                    );
                    if !acceptances.is_empty() {
                        let _ = write!(out, "; {}", acceptances.join(", "));
                    }
                    out.push(')');
                }
                out.push('\n');
            }
        }
        out
    }
}

/// `name` returns the name that the text prints for `entry`: its last part
/// in the listing of a directory, and its whole path otherwise, with a `/`
/// after a directory.
fn name(entry: &PackLsEntry, in_dir: bool) -> String {
    let base = if in_dir {
        entry.path.rsplit('/').next().unwrap_or(&entry.path)
    } else {
        &entry.path
    };
    if entry.directory {
        format!("{base}/")
    } else {
        base.to_string()
    }
}

/// `colour_of` returns the SGR parameters of the colour of `entry`: the
/// colour of an acceptance while its acceptance is `accepted`, whatever its
/// state, and otherwise the colour of its state, or none for an entry
/// without a state.
// sokf-implements: CLI-5_AC-28
// sokf-implements: CLI-5_AC-29
fn colour_of(entry: &PackLsEntry) -> Option<&'static str> {
    if entry.acceptance == Some(PackAcceptance::Accepted) {
        return Some(ACCEPTED);
    }
    Some(match entry.state? {
        PackLsState::Unchanged => "32",
        PackLsState::Edited => "33",
        PackLsState::Deleted => "31",
        PackLsState::Conflicted => "1;31",
    })
}

/// `paint` returns `text` in the colour of `entry`, when `colour` is set and
/// the entry has a colour.
// sokf-implements: CLI-5_AC-15
// sokf-implements: CLI-5_AC-16
// sokf-implements: CLI-5_AC-17
fn paint(text: &str, entry: &PackLsEntry, colour: bool) -> String {
    match colour_of(entry) {
        Some(c) if colour => format!("\x1b[{c}m{text}\x1b[0m"),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(state: Option<PackLsState>, acceptance: Option<PackAcceptance>) -> PackLsEntry {
        PackLsEntry {
            path: "/LICENSE".into(),
            directory: false,
            pack: state.map(|_| "house".into()),
            kind: state.map(|_| PackFileKind::File),
            state,
            acceptance,
            note: None,
            counts: None,
        }
    }

    // sokf-tests: CLI-5_AC-28
    // sokf-tests: CLI-5_AC-29
    #[test]
    fn an_accepted_entry_takes_the_colour_of_an_acceptance_whatever_its_state() {
        for state in PackLsState::ALL {
            let accepted = entry(Some(state), Some(PackAcceptance::Accepted));
            assert_eq!(colour_of(&accepted), Some(ACCEPTED), "{state:?}");
            let stale = entry(Some(state), Some(PackAcceptance::Stale));
            let none = entry(Some(state), None);
            assert_eq!(colour_of(&stale), colour_of(&none), "{state:?}");
            assert_ne!(colour_of(&none), Some(ACCEPTED), "{state:?}");
        }
        assert_eq!(colour_of(&entry(None, None)), None);
    }
}
