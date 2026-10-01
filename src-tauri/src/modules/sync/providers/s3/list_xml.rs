//! The listing scan: the S3 `ListBucketResult` body read by hand.
//!
//! A HAND SCAN AND NOT AN XML CRATE. The grammar consumed here is four element
//! names in a document the remote generates, and the root element is the
//! discriminator, so an HTML error page and a legitimately empty bucket cannot
//! be read as the same thing.

use super::super::http::normalize_etag;
use super::super::sigv4;
use crate::modules::sync::provider::{Entry, ProviderError};

/// The five predefined XML entities, expanded.
///
/// `&amp;` LAST, or an escaped entity would be double-expanded: a literal
/// ampersand-l-t written as five characters would come back as a less-than
/// sign rather than as the four characters it names.
fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The text of the first `tag` element in `xml`.
pub(super) fn tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(unescape(&xml[start..end]))
}

/// The inner text of every `tag` element in `xml`, in order.
fn blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(at) = rest.find(&open) {
        let from = at + open.len();
        let Some(len) = rest[from..].find(&close) else {
            break;
        };
        out.push(&rest[from..from + len]);
        rest = &rest[from + len + close.len()..];
    }
    out
}

/// One page of a listing: its rows, and the token for the next page.
///
/// A HAND SCAN AND NOT AN XML CRATE, matching the shape already chosen for the
/// other protocol this port will carry. The grammar consumed here is four
/// element names in a document the remote generates.
///
/// ITS DISCRIMINATOR IS THE ROOT ELEMENT, and that is the load-bearing part.
/// Keying on a row element instead would read an HTML error page and a
/// legitimately empty bucket as the same thing, and an empty listing where the
/// remote actually holds objects is a silent "your inventory is empty" that a
/// merge reads as a pile of deletes.
pub fn parse_list(bytes: &[u8]) -> Result<(Vec<Entry>, Option<String>), ProviderError> {
    let body = std::str::from_utf8(bytes)
        .map_err(|e| ProviderError::Malformed(format!("the listing did not decode: {e}")))?;
    if !body.contains("<ListBucketResult") {
        return Err(ProviderError::Malformed(
            "the listing carried no ListBucketResult element".to_string(),
        ));
    }

    let mut entries = Vec::new();
    for row in blocks(body, "Contents") {
        let key = tag_text(row, "Key").ok_or_else(|| {
            ProviderError::Malformed("a listing row carried no Key element".to_string())
        })?;
        // AN ABSENT ETAG IS REFUSED RATHER THAN DEFAULTED, because the empty
        // string is not an etag and would be handed straight back as a
        // condition on the next write - where it fails every conditional put,
        // silently and permanently. Same rule the object read applies.
        let etag = tag_text(row, "ETag")
            .map(|e| normalize_etag(&e))
            .filter(|e| !e.is_empty())
            .ok_or_else(|| {
                ProviderError::Malformed(format!("the listing row for \"{key}\" carried no etag"))
            })?;
        entries.push(Entry {
            key,
            etag,
            modified_at: tag_text(row, "LastModified")
                .as_deref()
                .and_then(sigv4::parse_iso8601_utc),
        });
    }

    let truncated = tag_text(body, "IsTruncated")
        .map(|v| v.trim().eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    // A PAGE THAT SAYS IT IS TRUNCATED AND THEN NAMES NO TOKEN IS A PROTOCOL
    // FAILURE, not the end of the listing. Reading it as the end is the
    // silent-loss shape the root-element check guards the empty case against:
    // a partial inventory returned as a complete one, which the merge above
    // reads as every absent record having been deleted.
    let next = if truncated {
        Some(
            tag_text(body, "NextContinuationToken")
                .filter(|t| !t.is_empty())
                .ok_or_else(|| {
                    ProviderError::Protocol(
                        "the listing said it was truncated and then named no continuation token"
                            .to_string(),
                    )
                })?,
        )
    } else {
        None
    };
    Ok((entries, next))
}

/// Fold one page into the accumulated listing, and say whether to ask for
/// another.
///
/// TAKES `prev`, THE TOKEN THAT PRODUCED THIS PAGE, because that is the only
/// way to notice a server handing back the same token forever - the
/// accumulated rows carry no token history. That case is `Protocol` and not
/// `Malformed`: the body decoded and parsed fine, and the server is the thing
/// that is broken.
pub fn accumulate(
    acc: &mut Vec<Entry>,
    prev: Option<&str>,
    page: (Vec<Entry>, Option<String>),
) -> Result<Option<String>, ProviderError> {
    let (entries, next) = page;
    acc.extend(entries);
    match next {
        None => Ok(None),
        Some(token) if Some(token.as_str()) == prev => Err(ProviderError::Protocol(
            "the listing repeated its continuation token, which never terminates".to_string(),
        )),
        Some(token) => Ok(Some(token)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- listing ----------------------------------------------------------

    /// A listing shaped the way a real one is, entity-escaped etags included.
    fn listing(rows: &str, truncated: bool, next: &str) -> String {
        let tail = if truncated {
            format!("<IsTruncated>true</IsTruncated><NextContinuationToken>{next}</NextContinuationToken>")
        } else {
            "<IsTruncated>false</IsTruncated>".to_string()
        };
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <ListBucketResult xmlns=\"http://s3.example/doc/2006-03-01/\">\
             <Name>subclave</Name><KeyCount>2</KeyCount>{rows}{tail}</ListBucketResult>"
        )
    }

    const ROWS: &str = "<Contents><Key>v1/obj/aa&amp;bb</Key>\
         <LastModified>2009-10-12T17:50:30.123Z</LastModified>\
         <ETag>&quot;9b2cf5&quot;</ETag><Size>42</Size></Contents>\
         <Contents><Key>v1/keyfile</Key>\
         <LastModified>2024-02-29T00:00:00Z</LastModified>\
         <ETag>&quot;0f1e2d&quot;</ETag><Size>310</Size></Contents>";

    #[test]
    fn a_listing_yields_each_key_with_its_etag_and_a_millisecond_stamp() {
        let (entries, next) = parse_list(listing(ROWS, false, "").as_bytes()).unwrap();
        assert_eq!(next, None);
        assert_eq!(
            entries,
            vec![
                Entry {
                    // The escaped ampersand comes back as one character, or a
                    // key with one in it would name a different object.
                    key: "v1/obj/aa&bb".to_string(),
                    etag: "9b2cf5".to_string(),
                    modified_at: Some(1_255_369_830_123),
                },
                Entry {
                    key: "v1/keyfile".to_string(),
                    etag: "0f1e2d".to_string(),
                    modified_at: Some(1_709_164_800_000),
                },
            ]
        );
    }

    #[test]
    fn an_empty_prefix_is_an_empty_listing_and_not_an_error() {
        let (entries, next) = parse_list(listing("", false, "").as_bytes()).unwrap();
        assert!(entries.is_empty());
        assert_eq!(next, None);
    }

    #[test]
    fn a_body_that_is_not_a_listing_is_refused_rather_than_read_as_empty() {
        // The failure this exists for: an empty list where the remote holds
        // objects is a silent "your inventory is empty", which a merge reads as
        // a pile of deletes. None of the row elements appears in either of
        // these, so keying on one of those would call both of them empty.
        for body in [
            "<html><body><h1>502 Bad Gateway</h1></body></html>",
            "",
            "<Error><Code>AccessDenied</Code></Error>",
        ] {
            assert!(
                matches!(
                    parse_list(body.as_bytes()),
                    Err(ProviderError::Malformed(_))
                ),
                "{body}"
            );
        }
        // And bytes that are not text at all.
        assert!(matches!(
            parse_list(&[0xff, 0xfe, 0x00, 0x01]),
            Err(ProviderError::Malformed(_))
        ));
    }

    #[test]
    fn a_truncated_listing_carries_its_continuation_token_and_a_complete_one_does_not() {
        let (_, next) = parse_list(listing(ROWS, true, "page-2").as_bytes()).unwrap();
        assert_eq!(next, Some("page-2".to_string()));
        let (_, none) = parse_list(listing(ROWS, false, "page-2").as_bytes()).unwrap();
        assert_eq!(none, None);
    }

    #[test]
    fn a_row_missing_its_key_or_its_etag_is_refused() {
        // An absent etag defaulted to the empty string is worse than a refusal:
        // it is handed back as a condition on the next write, where it fails
        // every conditional put and says nothing about why.
        for rows in [
            "<Contents><ETag>&quot;abc&quot;</ETag></Contents>",
            "<Contents><Key>v1/obj/abc</Key></Contents>",
            "<Contents><Key>v1/obj/abc</Key><ETag></ETag></Contents>",
            "<Contents><Key>v1/obj/abc</Key><ETag>&quot;&quot;</ETag></Contents>",
        ] {
            assert!(
                matches!(
                    parse_list(listing(rows, false, "").as_bytes()),
                    Err(ProviderError::Malformed(_))
                ),
                "{rows}"
            );
        }
    }

    #[test]
    fn a_page_that_claims_truncation_and_names_no_token_is_refused() {
        // THE PARTIAL-LOSS SHAPE, which the root-element check does not reach
        // because these bodies carry the root element perfectly well. Read as
        // the end of the listing, a truncated first page becomes a complete
        // inventory, and the merge above reads every record it omits as a
        // delete. `Protocol` and not `Malformed`: the body parsed fine, the
        // server is what is broken.
        for tail in [
            "<IsTruncated>true</IsTruncated>",
            "<IsTruncated>true</IsTruncated><NextContinuationToken></NextContinuationToken>",
        ] {
            let body =
                format!("<ListBucketResult><Name>subclave</Name>{ROWS}{tail}</ListBucketResult>");
            let got = parse_list(body.as_bytes());
            assert!(matches!(got, Err(ProviderError::Protocol(_))), "{got:?}");
        }
    }

    fn entry(key: &str) -> Entry {
        Entry {
            key: key.to_string(),
            etag: "e".to_string(),
            modified_at: None,
        }
    }

    #[test]
    fn pages_append_stop_and_refuse_to_spin() {
        let mut acc = Vec::new();

        // Page one asks for another.
        let next = accumulate(&mut acc, None, (vec![entry("a")], Some("p2".into()))).unwrap();
        assert_eq!(next, Some("p2".to_string()));

        // Page two APPENDS rather than replacing, and ends the listing.
        let next = accumulate(&mut acc, Some("p2"), (vec![entry("b")], None)).unwrap();
        assert_eq!(next, None);
        assert_eq!(
            acc.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"]
        );

        // A server handing back the token it was just given is a listing that
        // never terminates. `Protocol` and not `Malformed`: the body decoded
        // and parsed perfectly well.
        let spinning = accumulate(&mut acc, Some("p2"), (vec![entry("c")], Some("p2".into())));
        assert!(
            matches!(spinning, Err(ProviderError::Protocol(_))),
            "{spinning:?}"
        );
    }
}
