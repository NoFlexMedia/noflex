// SPDX-License-Identifier: AGPL-3.0-only
//
// Pointing search at an OpenAI-compatible embeddings API instead of the local
// embedder. Administrator only: it decides where the library's text is sent.

use std::sync::Arc;

use async_graphql::{Context, InputObject, Object};
use noflex_plugin_host::{PluginRegistry, RemoteEmbedder};
use noflex_storage::{Storage, domain::EmbeddingEndpoint};

use crate::access;

#[derive(Default)]
pub struct EmbeddingQuery;

#[Object]
impl EmbeddingQuery {
    /// The embeddings API in use, or null when embedding happens on this server.
    /// Administrator only.
    async fn embedding_endpoint(
        &self,
        ctx: &Context<'_>,
    ) -> async_graphql::Result<Option<EmbeddingEndpoint>> {
        access::require_admin(ctx)?;
        storage(ctx)?.embedding_endpoint().await.map_err(gql_err)
    }
}

#[derive(InputObject)]
pub struct EmbeddingEndpointInput {
    /// The API's base, such as `https://api.openai.com/v1`.
    url: String,
    /// Null keeps the saved key while `url` is unchanged. Empty removes it.
    api_key: Option<String>,
    model: String,
    /// Ask the model to shorten its vectors. Most models can't.
    request_dimensions: Option<u32>,
    #[graphql(default)]
    query_prefix: String,
    #[graphql(default)]
    document_prefix: String,
}

#[derive(Default)]
pub struct EmbeddingMutation;

#[Object]
impl EmbeddingMutation {
    /// Use an embeddings API. Sends it a test first and saves nothing if that
    /// fails. From then on every title, overview, credit and search text goes to
    /// it, and changing anything but the key reindexes the library. Administrator
    /// only.
    async fn set_embedding_endpoint(
        &self,
        ctx: &Context<'_>,
        input: EmbeddingEndpointInput,
    ) -> async_graphql::Result<EmbeddingEndpoint> {
        access::require_admin(ctx)?;
        let storage = storage(ctx)?;
        let url = input.url.trim().to_string();
        let parsed = url::Url::parse(&url)
            .map_err(|_| async_graphql::Error::new("that isn't a valid URL"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(async_graphql::Error::new("the URL must use http or https"));
        }
        let model = input.model.trim().to_string();
        if model.is_empty() {
            return Err(async_graphql::Error::new("a model is required"));
        }
        let saved = storage.embedding_endpoint().await.map_err(gql_err)?;
        let endpoint = RemoteEmbedder::probe(EmbeddingEndpoint {
            api_key: resolve_key(input.api_key, saved.as_ref(), &url),
            url,
            model,
            dimensions: 0,
            request_dimensions: input.request_dimensions,
            query_prefix: input.query_prefix,
            document_prefix: input.document_prefix,
        })
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        let embedder = RemoteEmbedder::new(endpoint.clone())
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        storage
            .set_embedding_endpoint(Some(&endpoint))
            .await
            .map_err(gql_err)?;
        registry(ctx)?.set_remote_embedder(Some(Arc::new(embedder)));
        Ok(endpoint)
    }

    /// Go back to embedding on this server, which reindexes the library unless
    /// vectors from before the endpoint are still around. Administrator only.
    async fn clear_embedding_endpoint(&self, ctx: &Context<'_>) -> async_graphql::Result<bool> {
        access::require_admin(ctx)?;
        storage(ctx)?
            .set_embedding_endpoint(None)
            .await
            .map_err(gql_err)?;
        registry(ctx)?.set_remote_embedder(None);
        Ok(true)
    }
}

/// A saved key stays with the URL it was given for, so editing the URL can't hand
/// it to a different host.
fn resolve_key(
    given: Option<String>,
    saved: Option<&EmbeddingEndpoint>,
    url: &str,
) -> Option<String> {
    match given {
        Some(key) => Some(key.trim().to_string()).filter(|key| !key.is_empty()),
        None => saved
            .filter(|saved| saved.url == url)
            .and_then(|saved| saved.api_key.clone()),
    }
}

fn storage<'a>(ctx: &'a Context<'_>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn registry<'a>(ctx: &'a Context<'_>) -> async_graphql::Result<&'a Arc<PluginRegistry>> {
    ctx.data::<Arc<PluginRegistry>>()
        .map_err(|_| async_graphql::Error::new("plugin host not configured"))
}

fn gql_err(error: noflex_storage::error::StorageError) -> async_graphql::Error {
    tracing::warn!(%error, "embedding settings: storage failed");
    async_graphql::Error::new("storage failed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> EmbeddingEndpoint {
        EmbeddingEndpoint {
            url: "https://a.example/v1".to_string(),
            api_key: Some("sk-saved".to_string()),
            model: "m".to_string(),
            dimensions: 3,
            request_dimensions: None,
            query_prefix: String::new(),
            document_prefix: String::new(),
        }
    }

    #[test]
    fn a_saved_key_never_follows_a_new_url() {
        let saved = saved();
        let same = "https://a.example/v1";
        let other = "https://b.example/v1";
        assert_eq!(
            resolve_key(None, Some(&saved), same).as_deref(),
            Some("sk-saved")
        );
        assert_eq!(resolve_key(None, Some(&saved), other), None);
        assert_eq!(
            resolve_key(Some("sk-new".into()), Some(&saved), other).as_deref(),
            Some("sk-new")
        );
        assert_eq!(resolve_key(Some("  ".into()), Some(&saved), same), None);
    }
}
