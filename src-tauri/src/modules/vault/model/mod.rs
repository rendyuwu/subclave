//! The vault data model and its webview-facing projections.
//!
//! The sealed records ([`Entry`], [`Group`], [`Tombstone`]) live only inside
//! the ciphertext; the projections ([`EntrySummary`], [`EntryDetail`],
//! [`EntryDraft`]) are what crosses the IPC boundary. Timestamps are Unix
//! milliseconds everywhere; the LWW stamp rule is [`stamp_next`].

mod device;
mod drafts;
mod records;
mod views;

pub use device::{DeviceState, SyncDevice};
pub use drafts::{normalize_tags, stamp_next, DraftCustomField, EntryDraft, GroupDraft};
pub(crate) use records::seed_reserved_groups;
pub use records::{
    CustomField, Entry, EntryColor, EntryUrl, EntryVersion, Group, MatchMode, Tombstone,
    TombstoneKind, VaultPayload, VersionReason, BROWSER_ID, ROOT_ID, TRASH_ID,
};
pub use views::{
    detail_of, host_of, summary_of, DetailCustomField, DetailVersion, EntryDetail, EntrySummary,
};
pub(crate) use views::{version_changed_names, version_of};
