//! Which hive-rig wrote a file, and how that compares with this one.
//!
//! Guides files and `.hive.json` rigs carry `hive_rig`, the version of the
//! program that wrote them. Every write stamps the running version, so the
//! field always names the last writer. Reading it never changes how a rig
//! builds: that is pinned by the guides `version` (see `components::Rules`).

use std::cmp::Ordering;

use serde::Serializer;

/// This program's version, as stamped into the files it writes.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// `major.minor.patch` of a version string; a pre-release or build suffix is
/// ignored, and missing parts count as 0.
pub fn parse(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = || -> Option<u64> {
        match parts.next() {
            Some(p) => p.parse().ok(),
            None => Some(0),
        }
    };
    let out = (next()?, next()?, next()?);
    parts.next().is_none().then_some(out)
}

/// How a file's `hive_rig` stamp compares with this program. `None` when the
/// file has no stamp (written before stamping) or the stamp can't be read.
pub fn compare(stamp: Option<&str>) -> Option<Ordering> {
    let theirs = parse(stamp?)?;
    let ours = parse(CURRENT)?;
    Some(theirs.cmp(&ours))
}

/// Serde `serialize_with` for a `hive_rig` field: always writes this program's
/// version, whatever the file was read with.
pub fn stamp<S: Serializer>(_: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(CURRENT)
}

/// A line about a file written by a newer hive-rig, if it was.
pub fn newer_note(what: &str, stamp: Option<&str>) -> Option<String> {
    (compare(stamp) == Some(Ordering::Greater)).then(|| {
        format!(
            "{what} was written by hive-rig {}, newer than this {CURRENT}; settings it added are ignored, so the rig may differ",
            stamp.unwrap_or_default()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(parse("0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse("1.2"), Some((1, 2, 0)));
        assert_eq!(parse("2.0.0-beta.1"), Some((2, 0, 0)));
        assert_eq!(parse("1.x"), None);
        assert_eq!(parse("1.2.3.4"), None);
        assert_eq!(compare(Some("0.0.9")), Some(Ordering::Less));
        assert_eq!(compare(Some(CURRENT)), Some(Ordering::Equal));
        assert_eq!(compare(Some("999.0.0")), Some(Ordering::Greater));
        assert_eq!(compare(None), None);
        assert!(newer_note("x", Some("999.0.0")).is_some());
        assert!(newer_note("x", Some(CURRENT)).is_none());
        assert!(newer_note("x", None).is_none());
    }
}
