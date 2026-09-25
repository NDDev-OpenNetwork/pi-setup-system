//! Marked user-global instruction attachment.
//!
//! The consumer owns the section text. This kernel splices it into one
//! target-relative file, preserves every byte outside the markers, and is
//! idempotent on identical bytes. Whole-setup apply/remove must not treat the
//! region as payload they are free to empty: [`preserve_in_replacement`] and
//! [`keep_region_on_withdraw`] are the two hooks.

use std::{io, path::Path};

/// Visible begin marker. HTML comments are refused — Claude strips them.
pub const BEGIN: &str = ":::begin-ai-stp";
/// Visible end marker. Inclusive of the following newline when present.
pub const END: &str = ":::end-ai-stp";

/// Refuse ambiguous ownership markers before a caller plans or writes bytes.
#[must_use]
pub fn markers_well_formed(existing: &str) -> bool {
    match (existing.find(BEGIN), existing.find(END)) {
        (None, None) => true,
        (Some(begin), Some(end)) => {
            begin < end
                && !existing[begin + BEGIN.len()..].contains(BEGIN)
                && !existing[end + END.len()..].contains(END)
        }
        _ => false,
    }
}

/// The marked region, including both markers, or `None` when either is missing
/// or they are out of order.
#[must_use]
pub fn extract(existing: &str) -> Option<&str> {
    let begin = existing.find(BEGIN)?;
    let end = existing.find(END)?;
    if end < begin {
        return None;
    }
    let mut end_at = end + END.len();
    if existing[end_at..].starts_with('\n') {
        end_at += 1;
    }
    Some(&existing[begin..end_at])
}

/// Replace the marked region or append it. Preserve every other byte.
#[must_use]
pub fn splice(existing: &str, section: &str) -> String {
    match extract(existing) {
        None if existing.is_empty() => section.to_owned(),
        None => {
            let mut held = existing.to_owned();
            if !held.ends_with('\n') {
                held.push('\n');
            }
            held.push_str(section);
            held
        }
        Some(held) => match existing.find(BEGIN) {
            None => existing.to_owned(),
            Some(begin) => format!(
                "{}{}{}",
                &existing[..begin],
                section,
                &existing[begin + held.len()..]
            ),
        },
    }
}

/// Pure region patch. `wrote` is false when the marked bytes already match.
///
/// An empty file takes `section` whole so bytes the consumer placed *before*
/// the markers (Cursor `alwaysApply` YAML) survive the first write. Later
/// calls splice only the marked region.
#[must_use]
pub fn patch(existing: &str, section: &str) -> (String, bool) {
    let desired = extract(section).unwrap_or(section);
    if let Some(current) = extract(existing)
        && current == desired
    {
        return (existing.to_owned(), false);
    }
    if existing.is_empty() {
        return (section.to_owned(), true);
    }
    (splice(existing, desired), true)
}

/// Keep an existing attachment when a setup writes the same path.
#[must_use]
pub fn preserve_in_replacement(existing: &str, incoming: &str) -> String {
    let Some(region) = extract(existing) else {
        return incoming.to_owned();
    };
    if extract(incoming).is_some() {
        let (updated, _) = patch(incoming, region);
        return updated;
    }
    let spliced = splice(incoming, region);
    let owned = owned_prefix(existing);
    if owned.is_empty() || spliced.starts_with("---\n") {
        return spliced;
    }
    format!("{owned}{spliced}")
}

/// After withdrawing a recorded file, keep the attachment remainder when one
/// existed. Returns `None` when the file had no attachment (caller deletes).
///
/// Cursor `alwaysApply` YAML sits before the markers. That prefix is the
/// initialize attachment, not setup payload, so it survives withdraw.
#[must_use]
pub fn keep_region_on_withdraw(existing: &str) -> Option<String> {
    let region = extract(existing)?;
    let owned = owned_prefix(existing);
    if owned.is_empty() {
        Some(region.to_owned())
    } else {
        Some(format!("{owned}{region}"))
    }
}

/// YAML frontmatter initialize placed before the markers, or empty.
///
/// Not "everything before BEGIN": a setup body can sit between the fence and
/// the region, and withdraw must still drop that body.
fn owned_prefix(existing: &str) -> &str {
    const OPEN: &str = "---\n";
    const CLOSE: &str = "\n---\n";
    if !existing.starts_with(OPEN) {
        return "";
    }
    let Some(rel) = existing[OPEN.len()..].find(CLOSE) else {
        return "";
    };
    let mut end = OPEN.len() + rel + CLOSE.len();
    if existing[end..].starts_with('\n') {
        end += 1;
    }
    if extract(&existing[end..]).is_none() && !existing[end..].contains(BEGIN) {
        return "";
    }
    &existing[..end]
}

/// True when `relative` is the attachment path this harness named.
#[must_use]
pub fn is_attachment(relative: &str, named: Option<&str>) -> bool {
    named.is_some_and(|path| path == relative)
}

/// UTF-8 text of a file, or empty only when it is missing.
pub fn read_utf8(path: &Path) -> io::Result<String> {
    match std::fs::read_to_string(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECTION: &str = ":::begin-ai-stp\nhello\n:::end-ai-stp\n";

    #[test]
    fn partial_reversed_or_duplicate_markers_are_ambiguous() {
        assert!(markers_well_formed("no attachment"));
        assert!(markers_well_formed(SECTION));
        assert!(!markers_well_formed(":::begin-ai-stp\n"));
        assert!(!markers_well_formed(":::end-ai-stp\n"));
        assert!(!markers_well_formed(":::end-ai-stp\n:::begin-ai-stp\n"));
        assert!(!markers_well_formed(&format!("{SECTION}{SECTION}")));
    }

    #[test]
    fn empty_file_receives_the_section() {
        let (updated, wrote) = patch("", SECTION);
        assert!(wrote);
        assert_eq!(updated, SECTION);
    }

    #[test]
    fn user_bytes_outside_the_markers_are_kept() {
        let (updated, wrote) = patch("keep-me\n", SECTION);
        assert!(wrote);
        assert_eq!(updated, format!("keep-me\n{SECTION}"));
    }

    #[test]
    fn identical_bytes_are_a_no_write() {
        let first = splice("keep-me\n", SECTION);
        let (second, wrote) = patch(&first, SECTION);
        assert!(!wrote);
        assert_eq!(second, first);
    }

    #[test]
    fn first_write_keeps_bytes_before_the_markers() {
        let section = "---\nalwaysApply: true\n---\n\n:::begin-ai-stp\nhello\n:::end-ai-stp\n";
        let (updated, wrote) = patch("", section);
        assert!(wrote);
        assert_eq!(updated, section);
        assert!(updated.starts_with("---\n"));

        let next = "---\nalwaysApply: true\n---\n\n:::begin-ai-stp\nchanged\n:::end-ai-stp\n";
        let (spliced, wrote_again) = patch(&updated, next);
        assert!(wrote_again);
        assert!(spliced.starts_with("---\n"));
        assert_eq!(spliced.matches("alwaysApply: true").count(), 1);
        assert_eq!(extract(&spliced), extract(next));
        assert_eq!(
            extract(&spliced),
            Some(":::begin-ai-stp\nchanged\n:::end-ai-stp\n")
        );
    }

    #[test]
    fn replacement_of_a_setup_file_keeps_the_region() {
        let existing = splice("old-setup\n", SECTION);
        let outgoing = preserve_in_replacement(&existing, "new-setup\n");
        assert!(outgoing.contains("new-setup"));
        assert_eq!(extract(&outgoing), Some(SECTION));
    }

    #[test]
    fn withdraw_leaves_the_region_and_drops_the_rest() {
        let existing = splice("setup-bytes\n", SECTION);
        assert_eq!(keep_region_on_withdraw(&existing).as_deref(), Some(SECTION));
        assert_eq!(keep_region_on_withdraw("just setup"), None);
    }

    #[test]
    fn yaml_frontmatter_survives_setup_replace_and_withdraw() {
        let section = "---\nalwaysApply: true\n---\n\n:::begin-ai-stp\nhello\n:::end-ai-stp\n";
        let (first, _) = patch("", section);
        let replaced = preserve_in_replacement(&first, "new-setup\n");
        assert!(replaced.starts_with("---\n"));
        assert_eq!(replaced.matches("alwaysApply: true").count(), 1);
        assert!(replaced.contains("new-setup"));
        assert_eq!(extract(&replaced), extract(section));

        let remainder = keep_region_on_withdraw(&replaced).unwrap_or_default();
        assert!(!remainder.is_empty());
        assert!(remainder.starts_with("---\n"));
        assert!(remainder.contains("alwaysApply: true"));
        assert!(!remainder.contains("new-setup"));
        assert_eq!(extract(&remainder), extract(section));
    }
}
