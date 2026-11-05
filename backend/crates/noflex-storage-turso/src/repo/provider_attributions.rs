// SPDX-License-Identifier: AGPL-3.0-only

use noflex_storage::{
    domain::{AttributionLogo, ProviderAttribution},
    error::StorageResult,
};
use turso::{Connection, Value};

use crate::{
    convert::{col_blob_opt, col_text, col_text_opt},
    error::map_turso,
};

const UPSERT: &str = "INSERT INTO provider_attributions \
    (provider, name, text, url, logo, logo_mime, logo_digest) \
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
    ON CONFLICT(provider) DO UPDATE SET \
        name = excluded.name, \
        text = excluded.text, \
        url = excluded.url, \
        logo = excluded.logo, \
        logo_mime = excluded.logo_mime, \
        logo_digest = excluded.logo_digest";

const LIST: &str = "SELECT provider, name, text, url, logo_mime, logo_digest \
    FROM provider_attributions ORDER BY provider";

const LOGO: &str = "SELECT logo, logo_mime FROM provider_attributions WHERE provider = ?1";

const DELETE: &str = "DELETE FROM provider_attributions WHERE provider = ?1";

fn text_or_null(value: &Option<String>) -> Value {
    value.clone().map(Value::Text).unwrap_or(Value::Null)
}

pub(crate) async fn upsert(
    conn: &Connection,
    attribution: &ProviderAttribution,
    logo: Option<&[u8]>,
) -> StorageResult<()> {
    let params: Vec<Value> = vec![
        Value::Text(attribution.provider.clone()),
        Value::Text(attribution.name.clone()),
        Value::Text(attribution.text.clone()),
        text_or_null(&attribution.url),
        logo.map(|bytes| Value::Blob(bytes.to_vec()))
            .unwrap_or(Value::Null),
        text_or_null(&attribution.logo_mime),
        text_or_null(&attribution.logo_digest),
    ];
    conn.execute(UPSERT, params).await.map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn delete(conn: &Connection, provider: &str) -> StorageResult<()> {
    conn.execute(DELETE, [Value::Text(provider.to_string())])
        .await
        .map_err(map_turso)?;
    Ok(())
}

pub(crate) async fn list(conn: &Connection) -> StorageResult<Vec<ProviderAttribution>> {
    let mut rows = conn.query(LIST, ()).await.map_err(map_turso)?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.map_err(map_turso)? {
        let provider = row.get_value(0).map_err(map_turso)?;
        let name = row.get_value(1).map_err(map_turso)?;
        let text = row.get_value(2).map_err(map_turso)?;
        let url = row.get_value(3).map_err(map_turso)?;
        let logo_mime = row.get_value(4).map_err(map_turso)?;
        let logo_digest = row.get_value(5).map_err(map_turso)?;
        out.push(ProviderAttribution {
            provider: col_text(&provider)?.to_string(),
            name: col_text(&name)?.to_string(),
            text: col_text(&text)?.to_string(),
            url: col_text_opt(&url)?,
            logo_mime: col_text_opt(&logo_mime)?,
            logo_digest: col_text_opt(&logo_digest)?,
        });
    }
    Ok(out)
}

pub(crate) async fn logo(
    conn: &Connection,
    provider: &str,
) -> StorageResult<Option<AttributionLogo>> {
    let mut rows = conn
        .query(LOGO, [Value::Text(provider.to_string())])
        .await
        .map_err(map_turso)?;
    let Some(row) = rows.next().await.map_err(map_turso)? else {
        return Ok(None);
    };
    let bytes = row.get_value(0).map_err(map_turso)?;
    let mime = row.get_value(1).map_err(map_turso)?;
    Ok(match (col_blob_opt(&bytes)?, col_text_opt(&mime)?) {
        (Some(bytes), Some(mime)) => Some(AttributionLogo { mime, bytes }),
        _ => None,
    })
}
