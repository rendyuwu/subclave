//! Plaintext CSV export in Bitwarden's layout, so the file re-imports here and
//! into Bitwarden.

use zeroize::Zeroizing;

use crate::modules::vault::model::{VaultPayload, ROOT_ID};
use crate::modules::vault::query::in_trash;

const HEADER: [&str; 11] = [
    "folder",
    "favorite",
    "type",
    "name",
    "notes",
    "fields",
    "reprompt",
    "login_uri",
    "login_username",
    "login_password",
    "login_totp",
];

/// One row per entry outside Trash, in payload order. History, tags, expiry,
/// icon, colour and the hidden flag do not survive the format.
pub(crate) fn csv_bytes(payload: &VaultPayload) -> Result<Zeroizing<Vec<u8>>, String> {
    let write_err = |e: csv::Error| format!("import: could not build the CSV file: {e}");
    let mut writer = csv::WriterBuilder::new().from_writer(Vec::new());
    writer.write_record(HEADER).map_err(write_err)?;
    for entry in &payload.entries {
        if in_trash(payload, &entry.group_id) {
            continue;
        }
        let fields = Zeroizing::new(
            entry
                .custom_fields
                .iter()
                .map(|f| format!("{}: {}", f.name, f.value))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let uris = entry
            .urls
            .iter()
            .map(|u| u.url.as_str())
            .collect::<Vec<_>>()
            .join(",");
        writer
            .write_record([
                group_path(payload, &entry.group_id).as_str(),
                if entry.favorite { "1" } else { "" },
                "login",
                &entry.title,
                &entry.notes,
                &fields,
                "0",
                &uris,
                &entry.username,
                &entry.password,
                entry.totp.as_deref().unwrap_or(""),
            ])
            .map_err(write_err)?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|e| format!("import: could not build the CSV file: {e}"))?;
    Ok(Zeroizing::new(bytes))
}

/// Group names from the top down to `group_id`, joined with `/`; an entry at
/// root gives `""`. Stops at root, a missing parent or a missing group, and
/// after `groups.len() + 1` steps, so a parent cycle cannot hang it.
fn group_path(payload: &VaultPayload, group_id: &str) -> String {
    let mut names = Vec::new();
    let mut current = Some(group_id);
    for _ in 0..=payload.groups.len() {
        let Some(id) = current.filter(|id| *id != ROOT_ID) else {
            break;
        };
        let Some(group) = payload.groups.iter().find(|g| g.id == id) else {
            break;
        };
        names.push(group.name.as_str());
        current = group.parent_id.as_deref();
    }
    names.reverse();
    names.join("/")
}
