// SPDX-License-Identifier: AGPL-3.0-only
//
// Provider credits: what a metadata plugin returns from `attribution()`,
// checked and kept in storage. Storage rather than the registry, because a
// title stays credited after its plugin is removed and clients ask for the
// logo by plugin id.

use noflex_storage::{Storage, domain::ProviderAttribution};
use sha2::{Digest, Sha256};

use crate::{
    LoadedPlugin, metadata_service::sniff_image, plugin_types::ProviderAttribution as Wit,
};

/// Past this it isn't a credit any more; such a plugin is left uncredited.
const MAX_TEXT_CHARS: usize = 300;
/// Logos are small; anything bigger is dropped.
const MAX_LOGO_BYTES: usize = 256 * 1024;

const SVG_MIME: &str = "image/svg+xml";

/// Read `plugin`'s attribution and store it, replacing what its last load
/// stored. One asking for no credit loses any it had. Failures are logged:
/// a plugin that can't say how to credit it still works.
pub(crate) async fn sync(plugin: &LoadedPlugin, storage: &dyn Storage) {
    let id = &plugin.id.0;
    let result = match plugin.call_attribution().await {
        Ok(Some(answer)) => match checked(id, &plugin.manifest.display_name, answer) {
            Some((row, logo)) => {
                storage
                    .upsert_provider_attribution(&row, logo.as_deref())
                    .await
            }
            None => return,
        },
        Ok(None) => storage.delete_provider_attribution(id).await,
        Err(error) => {
            tracing::warn!(plugin = %id, %error, "attribution: call failed");
            return;
        }
    };
    if let Err(error) = result {
        tracing::warn!(plugin = %id, %error, "attribution: not stored");
    }
}

/// `answer` as it will be stored, with its logo's bytes. `None` when the text
/// is unusable. A bad URL or logo is dropped and the text stands alone.
fn checked(
    provider: &str,
    name: &str,
    answer: Wit,
) -> Option<(ProviderAttribution, Option<Vec<u8>>)> {
    let text = answer.text.trim();
    if text.is_empty() || text.chars().count() > MAX_TEXT_CHARS {
        tracing::warn!(plugin = %provider, "attribution: text empty or too long; not credited");
        return None;
    }
    let url = answer.url.filter(|url| {
        let usable = url.starts_with("https://") || url.starts_with("http://");
        if !usable {
            tracing::warn!(plugin = %provider, "attribution: url is not http(s); dropped");
        }
        usable
    });
    let logo = answer.logo.and_then(|bytes| {
        let mime = logo_mime(&bytes).filter(|_| bytes.len() <= MAX_LOGO_BYTES);
        if mime.is_none() {
            tracing::warn!(plugin = %provider, bytes = bytes.len(),
                "attribution: logo too big or not a supported image; dropped");
        }
        mime.map(|mime| (mime, bytes))
    });
    let row = ProviderAttribution {
        provider: provider.to_string(),
        name: name.to_string(),
        text: text.to_string(),
        url,
        logo_mime: logo.as_ref().map(|(mime, _)| mime.to_string()),
        logo_digest: logo
            .as_ref()
            .map(|(_, bytes)| hex::encode(&Sha256::digest(bytes)[..12])),
    };
    Some((row, logo.map(|(_, bytes)| bytes)))
}

/// The content type of `bytes`, by what they hold rather than what a plugin says.
fn logo_mime(bytes: &[u8]) -> Option<&'static str> {
    if is_svg(bytes) {
        return Some(SVG_MIME);
    }
    Some(match sniff_image(bytes)? {
        "png" => "image/png",
        "jpg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    })
}

/// A UTF-8 document whose root element is `<svg`, past any XML declaration,
/// comments and doctype.
fn is_svg(bytes: &[u8]) -> bool {
    let Ok(document) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut rest = document.trim_start_matches('\u{feff}').trim_start();
    loop {
        if rest.starts_with("<svg")
            && rest[4..].starts_with(|c: char| c.is_whitespace() || c == '>' || c == '/')
        {
            return true;
        }
        let closing = if rest.starts_with("<?") {
            "?>"
        } else if rest.starts_with("<!--") {
            "-->"
        } else if rest.starts_with("<!") {
            ">"
        } else {
            return false;
        };
        let Some((_, after)) = rest.split_once(closing) else {
            return false;
        };
        rest = after.trim_start();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(text: &str, url: Option<&str>, logo: Option<&[u8]>) -> Wit {
        Wit {
            text: text.to_string(),
            url: url.map(str::to_string),
            logo: logo.map(<[u8]>::to_vec),
        }
    }

    const SVG: &[u8] = b"<?xml version=\"1.0\"?>\n<!-- mark --><svg viewBox=\"0 0 1 1\"></svg>";
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n....";

    #[test]
    fn logos_are_told_apart_by_content() {
        assert_eq!(logo_mime(SVG), Some(SVG_MIME));
        assert_eq!(logo_mime(b"<svg/>"), Some(SVG_MIME));
        assert_eq!(logo_mime(PNG), Some("image/png"));
        assert_eq!(logo_mime(b"<html><svg></svg></html>"), None);
        assert_eq!(logo_mime(b"<svgish></svgish>"), None);
        assert_eq!(logo_mime(b"\xFF\xFE not text"), None);
    }

    #[test]
    fn a_bad_url_or_logo_leaves_the_text_standing() {
        let (row, logo) = checked(
            "p",
            "P",
            answer(" Data from P. ", Some("javascript:alert(1)"), Some(b"nope")),
        )
        .expect("usable text");
        assert_eq!(row.text, "Data from P.");
        assert_eq!(row.url, None);
        assert_eq!(row.logo_mime, None);
        assert_eq!(row.logo_digest, None);
        assert_eq!(logo, None);
    }

    #[test]
    fn an_oversized_logo_is_dropped() {
        let mut huge = PNG.to_vec();
        huge.resize(MAX_LOGO_BYTES + 1, 0);
        let (row, logo) =
            checked("p", "P", answer("Data from P.", None, Some(&huge))).expect("usable text");
        assert_eq!(row.logo_mime, None);
        assert_eq!(logo, None);
    }

    #[test]
    fn a_new_logo_gets_a_new_digest() {
        let digest = |bytes: &[u8]| {
            checked("p", "P", answer("Data from P.", None, Some(bytes)))
                .expect("usable text")
                .0
                .logo_digest
                .expect("kept logo")
        };
        assert_ne!(digest(SVG), digest(PNG));
    }

    #[test]
    fn unusable_text_earns_no_credit() {
        assert!(checked("p", "P", answer("   ", None, None)).is_none());
        let long = "x".repeat(MAX_TEXT_CHARS + 1);
        assert!(checked("p", "P", answer(&long, None, None)).is_none());
    }
}
