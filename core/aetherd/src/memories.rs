//! Dial frequencies worth remembering — each a frequency and a name — kept in a file
//! beside the configuration, so the panel's dial list is the operator's own and survives
//! a reinstall like the stations heard do.
//!
//! The list starts as the frequency plan's proposals (`docs/user/frequency-plan.md`), one
//! dial per band with the plan's wording, and becomes whatever the operator makes of it:
//! `frequencies.set` on the control API replaces it whole. A radio keyed over CAT or
//! `rigctld` can be tuned to an entry with `frequency.set`; a serial line cannot tune
//! anything, and the list is still a list.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One remembered dial.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    /// The dial frequency, hertz.
    pub hz: u64,
    /// What the operator calls it: the band, the net, the friend.
    pub name: String,
}

#[derive(Serialize, Deserialize)]
struct File {
    schema: u32,
    memories: Vec<Memory>,
    /// The plan's proposals this list has been offered, by dial. A proposal added to the plan
    /// later — 6 m, after beta.67 — is added to a list once, and one the operator has since
    /// removed stays removed. A list written before this was kept offered the eight HF dials.
    /// An older version reading the file ignores the field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offered: Option<Vec<u64>>,
}

/// The plan's dials a list written before [`File::offered`] was kept had been offered: the
/// eight HF proposals of beta.66 and earlier.
const OFFERED_BEFORE_6M: [u64; 8] = [
    3_590_000, 7_101_000, 10_141_000, 14_107_000, 18_107_000, 21_094_000, 24_926_000, 28_126_000,
];

/// The most entries kept.
pub const LIMIT: usize = 200;
/// The dials a radio this modem could be attached to might have, hertz.
const LOWEST_HZ: u64 = 100_000;
const HIGHEST_HZ: u64 = 1_300_000_000;
const NAME_CHARS: usize = 60;

/// The frequency plan's proposals: the dials `docs/user/frequency-plan.md` suggests, marked
/// as proposals because nothing here is a standard yet.
#[must_use]
pub fn defaults() -> Vec<Memory> {
    let plan = [
        (3_590_000, "80 m — Aether calling (proposed)"),
        (7_101_000, "40 m — Aether calling (proposed)"),
        (10_141_000, "30 m — Aether calling at 500 Hz (proposed)"),
        (14_107_000, "20 m — Aether calling (proposed)"),
        (18_107_000, "17 m — Aether calling (proposed)"),
        (21_094_000, "15 m — Aether calling (proposed)"),
        (24_926_000, "12 m — Aether calling (proposed)"),
        (28_126_000, "10 m — Aether calling (proposed)"),
        (50_690_000, "6 m — Aether calling (proposed)"),
    ];
    plan.iter()
        .map(|&(hz, name)| Memory {
            hz,
            name: name.to_owned(),
        })
        .collect()
}

/// The list, and where it lives.
#[derive(Debug)]
pub struct Memories {
    entries: Vec<Memory>,
    path: Option<PathBuf>,
}

impl Memories {
    /// The list kept in `path`, loaded from it if it exists; the plan's proposals when it
    /// does not or cannot be read.
    #[must_use]
    pub fn open(path: Option<PathBuf>) -> Self {
        let entries = path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|text| serde_json::from_str::<File>(&text).ok())
            .filter(|file| file.schema == 1)
            .map_or_else(defaults, |file| {
                // a proposal the plan has gained since the list was last written joins it once
                let offered = file.offered.unwrap_or_else(|| OFFERED_BEFORE_6M.to_vec());
                let mut memories = file.memories;
                for proposal in defaults() {
                    if !offered.contains(&proposal.hz)
                        && !memories.iter().any(|m| m.hz == proposal.hz)
                    {
                        memories.push(proposal);
                    }
                }
                memories
            });
        let mut list = Self { entries, path };
        list.tidy();
        list
    }

    /// The entries, by frequency.
    #[must_use]
    pub fn entries(&self) -> &[Memory] {
        &self.entries
    }

    /// Where the list lives.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Replace the list. Names are trimmed, a frequency given twice keeps its last name,
    /// the result is sorted by frequency and capped.
    ///
    /// # Errors
    /// For a frequency no radio has a dial for, or a name that is too long.
    pub fn replace(&mut self, entries: Vec<Memory>) -> Result<(), String> {
        for entry in &entries {
            if !(LOWEST_HZ..=HIGHEST_HZ).contains(&entry.hz) {
                return Err(format!("{} Hz is not a dial frequency", entry.hz));
            }
            if entry.name.trim().chars().count() > NAME_CHARS {
                return Err(format!("a name is at most {NAME_CHARS} characters"));
            }
        }
        self.entries = entries;
        self.tidy();
        Ok(())
    }

    fn tidy(&mut self) {
        let mut tidy: Vec<Memory> = Vec::new();
        for entry in self.entries.drain(..) {
            let name = entry.name.trim().to_owned();
            if let Some(existing) = tidy.iter_mut().find(|m| m.hz == entry.hz) {
                existing.name = name;
            } else {
                tidy.push(Memory { hz: entry.hz, name });
            }
        }
        tidy.sort_by_key(|m| m.hz);
        tidy.truncate(LIMIT);
        self.entries = tidy;
    }

    /// Write the list, whole, to a temporary file renamed into place.
    ///
    /// # Errors
    /// If the file cannot be written.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let file = File {
            schema: 1,
            memories: self.entries.clone(),
            offered: Some(defaults().iter().map(|m| m.hz).collect()),
        };
        let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(&temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_list_is_the_plans_proposals_and_an_edited_one_is_its_own() {
        let dir = std::env::temp_dir().join(format!("aether-memories-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("frequencies.json");
        let mut list = Memories::open(Some(path.clone()));
        assert_eq!(list.entries().len(), 9);
        assert_eq!(list.entries()[3].hz, 14_107_000);
        assert_eq!(list.entries()[8].hz, 50_690_000, "6 m, the last proposal");
        assert!(
            !path.exists(),
            "nothing is written until the operator edits"
        );

        list.replace(vec![
            Memory {
                hz: 7_101_000,
                name: "  40 m net  ".into(),
            },
            Memory {
                hz: 14_107_000,
                name: "first".into(),
            },
            Memory {
                hz: 14_107_000,
                name: "20 m, the last name wins".into(),
            },
        ])
        .expect("valid");
        assert_eq!(list.entries().len(), 2);
        assert_eq!(list.entries()[0].name, "40 m net");
        assert_eq!(list.entries()[1].name, "20 m, the last name wins");
        list.save().expect("written");
        let again = Memories::open(Some(path.clone()));
        assert_eq!(again.entries(), list.entries());

        assert!(
            list.replace(vec![Memory {
                hz: 12,
                name: "not a dial".into()
            }])
            .is_err()
        );
        assert!(
            list.replace(vec![Memory {
                hz: 7_101_000,
                name: "x".repeat(61)
            }])
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_list_from_before_6m_gains_the_6m_proposal_once_and_keeps_what_was_removed() {
        // beta.67 and earlier wrote no `offered`: the list had been offered the eight HF
        // dials, and its operator had removed two of them
        let dir = std::env::temp_dir().join(format!("aether-memories-6m-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("frequencies.json");
        let old = r#"{"schema": 1, "memories": [
            {"hz": 7101000, "name": "40 m net"},
            {"hz": 14107000, "name": "20 m"}]}"#;
        std::fs::write(&path, old).expect("written");
        let mut list = Memories::open(Some(path.clone()));
        let dials: Vec<u64> = list.entries().iter().map(|m| m.hz).collect();
        assert_eq!(
            dials,
            [7_101_000, 14_107_000, 50_690_000],
            "6 m joins, 80 m does not return"
        );
        // removed now, it stays removed
        let without: Vec<Memory> = list
            .entries()
            .iter()
            .filter(|m| m.hz != 50_690_000)
            .cloned()
            .collect();
        list.replace(without).expect("valid");
        list.save().expect("written");
        let again = Memories::open(Some(path.clone()));
        assert!(again.entries().iter().all(|m| m.hz != 50_690_000));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
