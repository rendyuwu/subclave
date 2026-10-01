//! The DAV XML scan: a multi-status body.

use super::super::http::normalize_etag;
use super::super::xml_tree::{tree, Element};
use super::dav_time::parse_http_date;
use crate::modules::sync::provider::{Entry, ProviderError};

// --- the scan -------------------------------------------------------------
//
// The document walk is shared plumbing, in
// `src-tauri/src/modules/sync/providers/xml_tree.rs`; what stays here is the
// lookup, which is DAV's own.
//
// IT MATCHES ON THE LOCAL NAME, which is the one thing that cannot be shared
// with the other backend. That protocol's listing carries no namespace
// prefixes, so searching for a literal `<Key>` is exact; a multi-status body
// has prefixes and the server picks them - one common server emits `D:href`
// and `lp1:getetag` in the same document, others emit `d:href`, and a body
// with a default namespace and no prefixes at all is legal too.
//
// The ceiling, named: matching a local name across namespaces means an element
// from some other namespace sharing a local name would be read. In a
// multi-status body that is not a shape any server produces, and closing it
// properly means a namespace-resolving parser, for namespaces neither this
// provider nor the other one draws anything from.

/// The path half of an `href`, which a server may spell as a whole URL or as an
/// absolute path.
///
/// Both spellings are handled per row even though a server has to pick one and
/// stay with it across a response, because handling both costs nothing and
/// relying on that consistency buys nothing.
///
/// THE TEXT ARRIVES UNESCAPED, from the reader: the XML escaping is the outer
/// layer and is already gone, so nothing here may expand an ampersand a second
/// time.
fn href_path(href: &str) -> String {
    let href = href.trim();
    match url::Url::parse(href) {
        Ok(url) => url.path().to_string(),
        // Not a whole URL, so it is the absolute-path spelling - which may
        // carry a query the path does not include.
        Err(_) => href.split('?').next().unwrap_or(href).to_string(),
    }
}

fn segments(path: &str) -> Vec<&str> {
    path.split('/').filter(|s| !s.is_empty()).collect()
}

/// Whether a status line carries a success code.
fn is_ok_status(line: &str) -> bool {
    line.split_whitespace()
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .is_some_and(|code| (200..300).contains(&code))
}

/// One property of one row, read ONLY from a group the server reported success
/// for.
///
/// THE GATE IS THE POINT OF THIS FUNCTION. A property the server will not
/// supply comes back as an EMPTY element inside a group whose own status is a
/// failure - which is routine for an etag on a collection - so a scan of the
/// whole row would find the empty one and store an empty etag. That failure is
/// permanent and silent: the empty etag stored by one pull equals the empty
/// etag the next pull reads, so the record is skipped on every later pull even
/// after its contents change, for the life of the prefix.
///
/// Gated on success rather than on "not missing", because a server may decline
/// a property with a forbidden status just as readily as with a missing one.
///
/// THE VALUE IS TAKEN AS THE READER SPELLED IT, with the five predefined
/// entities and any character reference resolved, and an etag is the reason
/// that matters. A server is free to escape a quotation mark in a text node and
/// at least one common one does, so the listing would carry an etag spelled
/// differently from the one the same server puts in a response HEADER - and
/// those two are compared against each other by the layer above. Keeping the
/// escaped spelling makes that comparison fail on every row forever, which
/// turns every pull into a full download of the whole inventory.
fn prop_of(row: &Element, name: &str) -> Option<String> {
    for group in row.all("propstat") {
        let Some(status) = group.find("status") else {
            continue;
        };
        if !is_ok_status(&status.text) {
            continue;
        }
        let Some(props) = group.find("prop") else {
            continue;
        };
        if let Some(value) = props.find(name) {
            return Some(value.text.clone());
        }
    }
    None
}

fn is_collection(row: &Element) -> bool {
    row.all("resourcetype")
        .iter()
        .any(|kind| kind.find("collection").is_some())
}

/// The rows of a listing.
///
/// `request_path` is the server-side path that was asked for, and is read only
/// for its SEGMENT COUNT: a child of the requested collection has exactly one
/// segment more. Counting separators rather than comparing the two paths as
/// strings is what makes the self-entry every listing contains drop out, and it
/// survives the server spelling the user's prefix with a different escaping
/// than we sent, since the number of separators is invariant under any legal
/// one. A trailing slash on a collection stops mattering for the same reason.
///
/// `prefix` is the key prefix as the caller asked for it, ending in a
/// separator, and the key of a row is that prefix plus the row's last segment
/// TAKEN VERBATIM. Not decoded: the names the layer above composes are hex, so
/// no server escapes one, and the other half of the key comes from the request
/// rather than from the response - so a decoder here would be code that
/// provably never changes a byte. The residue, named: this couples the provider
/// to the naming scheme one layer up, and if that scheme ever emits a character
/// needing an escape, this is where it is noticed.
///
/// ITS DISCRIMINATOR IS THE ROOT ELEMENT, and that is the load-bearing part.
/// Keying on a row element instead would read an HTML error page and a
/// legitimately empty collection as the same thing. An empty listing where the
/// remote actually holds objects is a silent "your inventory is empty": every
/// remote record reads as one this device alone has, so the pass re-uploads the
/// whole inventory over whatever is already there and reports nothing wrong.
pub fn parse_multistatus(
    bytes: &[u8],
    request_path: &str,
    prefix: &str,
) -> Result<Vec<Entry>, ProviderError> {
    let body = std::str::from_utf8(bytes)
        .map_err(|e| ProviderError::Malformed(format!("the listing did not decode: {e}")))?;
    let roots = tree(body)?;
    let Some(root) = roots.first().filter(|root| root.name == "multistatus") else {
        return Err(ProviderError::Malformed(
            "the listing carried no multistatus element".to_string(),
        ));
    };

    let depth = segments(request_path).len();
    let mut entries = Vec::new();
    for row in root.all("response") {
        let href = row.find("href").ok_or_else(|| {
            ProviderError::Malformed("a listing row carried no href element".to_string())
        })?;
        let path = href_path(&href.text);
        let found = segments(&path);
        if found.len() != depth + 1 {
            continue;
        }
        // A belt beside the segment count: the requested collection's own entry
        // is already gone by depth, and a nested collection cannot appear at
        // this depth in a flat namespace, but neither of those is this
        // provider's to guarantee.
        if is_collection(row) {
            continue;
        }
        // A MEMBER THE SERVER WOULD NOT DESCRIBE AT ALL is skipped, where a
        // member it described badly is refused below. The two are different
        // shapes and want opposite answers: a row carrying an href and a bare
        // status, with no property groups whatsoever, is what a server sends
        // for a file it could not stat - a permissions refusal, a broken link -
        // and refusing the whole listing over one of those would strand the
        // entire prefix on every pull while every other row was readable. A row
        // that DOES carry property groups and still yields no usable etag is a
        // server contradicting itself, and that is refused.
        if row.find("propstat").is_none() {
            continue;
        }
        let key = format!("{prefix}{}", found[found.len() - 1]);
        // AN ABSENT OR EMPTY ETAG IS REFUSED RATHER THAN DEFAULTED, the same
        // rule the object read applies: the empty string is not an etag, and
        // storing one makes the row match itself forever.
        let etag = prop_of(row, "getetag")
            .map(|e| normalize_etag(&e))
            .filter(|e| !e.is_empty())
            .ok_or_else(|| {
                ProviderError::Malformed(format!("the listing row for \"{key}\" carried no etag"))
            })?;
        entries.push(Entry {
            key,
            etag,
            modified_at: prop_of(row, "getlastmodified")
                .as_deref()
                .and_then(parse_http_date),
        });
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the listing ------------------------------------------------------

    /// One row of a fixture, before a namespace spelling is chosen for it.
    struct Row {
        href: &'static str,
        etag: &'static str,
        modified: &'static str,
        collection: bool,
    }

    fn file(href: &'static str, etag: &'static str) -> Row {
        Row {
            href,
            etag,
            modified: "Mon, 12 Jan 1998 09:25:56 GMT",
            collection: false,
        }
    }

    /// A listing body in one of the namespace spellings a server may choose:
    /// `"D"` and `"d"` for a prefixed document, `""` for a default namespace
    /// with no prefixes at all.
    fn fixture(ns: &str, rows: &[Row]) -> String {
        let (decl, p) = if ns.is_empty() {
            (" xmlns=\"DAV:\"".to_string(), String::new())
        } else {
            (format!(" xmlns:{ns}=\"DAV:\""), format!("{ns}:"))
        };
        let mut out = format!("<?xml version=\"1.0\"?><{p}multistatus{decl}>");
        for row in rows {
            let kind = if row.collection {
                format!("<{p}collection/>")
            } else {
                String::new()
            };
            out.push_str(&format!(
                "<{p}response><{p}href>{}</{p}href><{p}propstat><{p}prop>\
                 <{p}getetag>{}</{p}getetag><{p}getlastmodified>{}</{p}getlastmodified>\
                 <{p}resourcetype>{kind}</{p}resourcetype>\
                 </{p}prop><{p}status>HTTP/1.1 200 OK</{p}status></{p}propstat></{p}response>",
                row.href, row.etag, row.modified
            ));
        }
        out.push_str(&format!("</{p}multistatus>"));
        out
    }

    const REQUEST_PATH: &str = "/remote.php/dav/files/rendi/vault/v1/obj/";
    const PREFIX: &str = "vault/v1/obj/";

    fn parse(body: &str) -> Result<Vec<Entry>, ProviderError> {
        parse_multistatus(body.as_bytes(), REQUEST_PATH, PREFIX)
    }

    #[test]
    fn the_namespace_prefix_a_server_chose_does_not_reach_the_result() {
        let rows = || {
            vec![
                Row {
                    href: "/remote.php/dav/files/rendi/vault/v1/obj/",
                    etag: "\"self\"",
                    modified: "Mon, 12 Jan 1998 09:25:56 GMT",
                    collection: true,
                },
                file("/remote.php/dav/files/rendi/vault/v1/obj/ab12", "\"one\""),
                file("/remote.php/dav/files/rendi/vault/v1/obj/cd34", "W/\"two\""),
            ]
        };
        let expected = vec![
            Entry {
                key: "vault/v1/obj/ab12".to_string(),
                etag: "one".to_string(),
                modified_at: Some(884_597_156_000),
            },
            Entry {
                key: "vault/v1/obj/cd34".to_string(),
                etag: "two".to_string(),
                modified_at: Some(884_597_156_000),
            },
        ];
        // Byte-identical across all three spellings, and the requested
        // collection's own entry is in none of them.
        for ns in ["D", "d", ""] {
            assert_eq!(parse(&fixture(ns, &rows())).unwrap(), expected, "ns {ns:?}");
        }
    }

    #[test]
    fn a_trailing_slash_on_a_row_does_not_change_its_key() {
        // Named by the issue this file answers. Discharged by the segment
        // count rather than by a case of its own: empty segments are dropped,
        // so only the requested collection's own entry can carry one at all.
        let with = fixture(
            "d",
            &[file(
                "/remote.php/dav/files/rendi/vault/v1/obj/ab12/",
                "\"x\"",
            )],
        );
        let without = fixture(
            "d",
            &[file(
                "/remote.php/dav/files/rendi/vault/v1/obj/ab12",
                "\"x\"",
            )],
        );
        let rows = parse(&with).unwrap();
        // Not vacuous: both sides really produced the row, rather than both
        // producing nothing.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows, parse(&without).unwrap());
    }

    #[test]
    fn an_href_spelled_as_a_whole_url_or_carrying_a_query_yields_the_same_key() {
        let path = fixture(
            "d",
            &[file(
                "/remote.php/dav/files/rendi/vault/v1/obj/ab12",
                "\"x\"",
            )],
        );
        let absolute = fixture(
            "d",
            &[file(
                "https://cloud.example/remote.php/dav/files/rendi/vault/v1/obj/ab12",
                "\"x\"",
            )],
        );
        let queried = fixture(
            "d",
            &[file(
                "/remote.php/dav/files/rendi/vault/v1/obj/ab12?v=2",
                "\"x\"",
            )],
        );
        let expected = parse(&path).unwrap();
        assert_eq!(expected.len(), 1);
        assert_eq!(parse(&absolute).unwrap(), expected);
        assert_eq!(parse(&queried).unwrap(), expected);
    }

    #[test]
    fn an_escaped_value_is_expanded_before_it_is_stored() {
        // At least one common server escapes a quotation mark in a text node,
        // and the same server puts the unescaped spelling in a response HEADER.
        // Those two are compared against each other one layer up, so storing
        // the escaped form would make the comparison fail on every row forever
        // and turn every pull into a full download of the inventory.
        let body = "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\"><d:response>\
             <d:href>/remote.php/dav/files/rendi/vault/v1/obj/ab12</d:href>\
             <d:propstat><d:prop><d:getetag>&quot;66a1f2&quot;</d:getetag></d:prop>\
             <d:status>HTTP/1.1 200 OK</d:status></d:propstat>\
             </d:response></d:multistatus>";
        let entries = parse(body).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].etag, "66a1f2");
    }

    #[test]
    fn a_member_the_server_would_not_describe_is_skipped_rather_than_fatal() {
        // A row carrying an href and a bare status, with no property groups at
        // all, is what a server sends for a member it could not stat. Refusing
        // the listing over one of those would strand the whole prefix on every
        // pull while every other row was perfectly readable.
        let body = "<?xml version=\"1.0\"?><D:multistatus xmlns:D=\"DAV:\"><D:response>\
             <D:href>/remote.php/dav/files/rendi/vault/v1/obj/bad1</D:href>\
             <D:status>HTTP/1.1 403 Forbidden</D:status></D:response><D:response>\
             <D:href>/remote.php/dav/files/rendi/vault/v1/obj/ab12</D:href>\
             <D:propstat><D:prop><D:getetag>\"good\"</D:getetag></D:prop>\
             <D:status>HTTP/1.1 200 OK</D:status></D:propstat>\
             </D:response></D:multistatus>";
        let entries = parse(body).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "vault/v1/obj/ab12");
    }

    #[test]
    fn a_collection_sitting_among_the_objects_is_not_one_of_them() {
        // Not the requested collection's own entry, which the segment count
        // already drops - a nested one, at the same depth as the objects,
        // which somebody made in the file browser. Read as an object it would
        // be fetched and then quarantined on every pull.
        let body = fixture(
            "d",
            &[
                Row {
                    href: "/remote.php/dav/files/rendi/vault/v1/obj/notes/",
                    etag: "\"dir\"",
                    modified: "Mon, 12 Jan 1998 09:25:56 GMT",
                    collection: true,
                },
                file("/remote.php/dav/files/rendi/vault/v1/obj/ab12", "\"one\""),
            ],
        );
        let entries = parse(&body).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "vault/v1/obj/ab12");
    }

    /// A row carrying two property groups: one the server refused, holding an
    /// empty value, and one it answered, holding the real one.
    fn two_group_row(failing_status: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\"><d:response>\
             <d:href>/remote.php/dav/files/rendi/vault/v1/obj/ab12</d:href>\
             <d:propstat><d:prop><d:getetag/></d:prop>\
             <d:status>{failing_status}</d:status></d:propstat>\
             <d:propstat><d:prop><d:getetag>\"real\"</d:getetag>\
             <d:getlastmodified>Mon, 12 Jan 1998 09:25:56 GMT</d:getlastmodified></d:prop>\
             <d:status>HTTP/1.1 200 OK</d:status></d:propstat>\
             </d:response></d:multistatus>"
        )
    }

    #[test]
    fn a_property_is_read_only_from_a_group_the_server_answered() {
        // The refused group holds an empty etag. Storing that would make the
        // row match itself on every later pull, so the record would be skipped
        // forever even after its contents changed.
        for status in ["HTTP/1.1 404 Not Found", "HTTP/1.1 403 Forbidden"] {
            let entries = parse(&two_group_row(status)).unwrap();
            assert_eq!(entries.len(), 1, "{status}");
            assert_eq!(entries[0].etag, "real", "{status}");
            assert_eq!(entries[0].modified_at, Some(884_597_156_000), "{status}");
        }
    }

    #[test]
    fn a_row_whose_only_answer_is_a_refusal_is_malformed_rather_than_empty() {
        let body = "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\"><d:response>\
             <d:href>/remote.php/dav/files/rendi/vault/v1/obj/ab12</d:href>\
             <d:propstat><d:prop><d:getetag/></d:prop>\
             <d:status>HTTP/1.1 403 Forbidden</d:status></d:propstat>\
             </d:response></d:multistatus>";
        let err = parse(body).expect_err("an etagless row is refused");
        assert!(
            matches!(&err, ProviderError::Malformed(m) if m.contains("ab12")),
            "{err:?}"
        );
    }

    #[test]
    fn a_body_that_is_not_a_listing_is_malformed_and_an_empty_listing_is_not() {
        // An error page read as an empty inventory is a pile of deletes.
        let page = b"<html><body><h1>404 Not Found</h1></body></html>";
        let err = parse_multistatus(page, REQUEST_PATH, PREFIX)
            .expect_err("an html page is not a listing");
        assert!(matches!(err, ProviderError::Malformed(_)), "{err:?}");

        // A collection holding nothing still answers with its own entry.
        let only_self = fixture(
            "d",
            &[Row {
                href: "/remote.php/dav/files/rendi/vault/v1/obj/",
                etag: "\"self\"",
                modified: "Mon, 12 Jan 1998 09:25:56 GMT",
                collection: true,
            }],
        );
        assert_eq!(parse(&only_self).unwrap(), vec![]);
    }

    #[test]
    fn a_listing_row_with_no_href_is_malformed() {
        let body = "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\"><d:response>\
             <d:propstat><d:prop><d:getetag>\"x\"</d:getetag></d:prop>\
             <d:status>HTTP/1.1 200 OK</d:status></d:propstat>\
             </d:response></d:multistatus>";
        assert!(matches!(parse(body), Err(ProviderError::Malformed(_))));
    }

    // --- tolerance --------------------------------------------------------
    //
    // The reader is loosened where the hand scanners this replaced scanned text
    // and could not fail on well-formedness, so a body the server emits with
    // one bad byte still yields its rows rather than failing the whole pull.
    // One test per class of bad byte.

    #[test]
    fn a_dangling_ampersand_still_yields_the_rows() {
        let body = "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\"><d:response>\
             <d:href>/remote.php/dav/files/rendi/vault/v1/obj/ab12</d:href>\
             <d:propstat><d:prop><d:getetag>\"a&b\"</d:getetag></d:prop>\
             <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>";
        let entries = parse(body).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "vault/v1/obj/ab12");
        assert_eq!(entries[0].etag, "a&b");
    }

    #[test]
    fn a_mismatched_end_tag_still_yields_the_rows() {
        let body = "<?xml version=\"1.0\"?><d:multistatus xmlns:d=\"DAV:\"><d:response>\
             <d:href>/remote.php/dav/files/rendi/vault/v1/obj/ab12</d:href>\
             <d:propstat><d:prop><d:getetag>\"x\"</d:getetag></d:prop>\
             <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:responseX>\
             </d:multistatus>";
        let entries = parse(body).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "vault/v1/obj/ab12");
    }

    #[test]
    fn a_stray_end_tag_still_yields_the_rows() {
        let body = format!(
            "{}</nothing>",
            fixture(
                "d",
                &[file(
                    "/remote.php/dav/files/rendi/vault/v1/obj/ab12",
                    "\"x\""
                )]
            )
        );
        let entries = parse(&body).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "vault/v1/obj/ab12");
    }
}
