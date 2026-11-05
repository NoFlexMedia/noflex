// SPDX-License-Identifier: AGPL-3.0-only
//
// Client-facing library search. Hybrid by default: lexical matches
// fused with embedding similarity (RRF) in the storage layer. With no
// embedder plugin loaded, hybrid silently degrades to lexical-only;
// an explicit `VECTOR` request instead errors so the client knows
// semantic search is unavailable. Reads are not auth-gated,
// consistent with the rest of the browse surface.

use std::sync::Arc;

use async_graphql::{Context, Object, connection::Connection};
use noflex_core::WorkKind;
use noflex_plugin_host::{EmbedInput, PluginRegistry, embed_inputs};
use noflex_storage::{HybridQuery, SearchHit, SearchMode, Storage};

#[derive(Default)]
pub struct SearchQuery;

#[Object]
impl SearchQuery {
    /// Search every library by title, credited people and overview text
    /// and, when an embedder plugin is loaded, semantic similarity. Hits are
    /// score-ranked, best first; cursors are rank positions, so paging
    /// re-runs the search and reaches at most the top 100 hits. `first`
    /// defaults to 20, max 100. `kinds` omitted or empty searches all
    /// work kinds.
    #[graphql(complexity = "crate::paging::cost(first, 20, 100, child_complexity)")]
    async fn search(
        &self,
        ctx: &Context<'_>,
        text: String,
        #[graphql(default)] mode: SearchMode,
        kinds: Option<Vec<WorkKind>>,
        first: Option<i32>,
        after: Option<String>,
    ) -> async_graphql::Result<Connection<String, SearchHit>> {
        let window = crate::paging::RankWindow::new(first, after, 20, 100, MAX_SEARCH_DEPTH)?;
        if window.first == 0 {
            return Ok(crate::paging::empty());
        }
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        let request_user = crate::access::request_user(ctx)?;
        let access_user_id = request_user.access_user_id();
        let kinds = kinds.unwrap_or_default();
        let limit = window.fetch().min(MAX_SEARCH_DEPTH) as u32;
        let hits = match mode {
            SearchMode::Lexical => storage
                .search_lexical(&text, &kinds, access_user_id, limit)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string())),
            SearchMode::Vector => {
                let embedded = embed_query(ctx, &text)
                    .await?
                    .ok_or_else(|| async_graphql::Error::new("no embedder plugin is available"))?;
                let hits = storage
                    .search_vector(
                        &embedded.namespace,
                        &embedded.vector,
                        &kinds,
                        access_user_id,
                        limit,
                    )
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
                Ok(hits
                    .into_iter()
                    .map(|hit| SearchHit {
                        work: hit.work,
                        access_checked: true,
                        score: hit.similarity,
                        matched_fields: vec!["embedding".to_string()],
                    })
                    .collect())
            }
            SearchMode::Hybrid => {
                // Best-effort embed: a missing registry, an unloaded
                // embedder, or a failing plugin call all degrade to
                // vectorless hybrid, which the storage layer treats
                // as lexical-only.
                let embedded = match embed_query(ctx, &text).await {
                    Ok(embedded) => embedded,
                    Err(e) => {
                        tracing::warn!(error = %e.message, "embedder failed; hybrid search degrading to lexical-only");
                        None
                    }
                };
                let (vector_namespace, query_vector) = match embedded {
                    Some(e) => (e.namespace, Some(e.vector)),
                    None => (String::new(), None),
                };
                storage
                    .search_hybrid(&HybridQuery {
                        access_user_id,
                        text,
                        query_vector,
                        vector_namespace,
                        kinds,
                        limit,
                        mode: SearchMode::Hybrid,
                    })
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))
            }
        }?;
        Ok(window.connection(
            hits.into_iter()
                .map(|mut hit| {
                    hit.access_checked = true;
                    hit
                })
                .collect(),
        ))
    }
}

/// Deepest rank a search page reaches; storage is never asked for more.
const MAX_SEARCH_DEPTH: usize = 100;

struct EmbeddedQuery {
    namespace: String,
    vector: Vec<f32>,
}

/// Embed one query text with the highest-priority embedder plugin.
/// `Ok(None)` covers both "no registry in the schema context" and
/// "no embedder loaded"; `Err` is a failing plugin call.
async fn embed_query(
    ctx: &Context<'_>,
    text: &str,
) -> async_graphql::Result<Option<EmbeddedQuery>> {
    let Ok(registry) = ctx.data::<Arc<PluginRegistry>>() else {
        return Ok(None);
    };
    let outcome = embed_inputs(registry, vec![EmbedInput::Query(text.to_string())])
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    Ok(outcome.and_then(|o| {
        o.vectors.into_iter().next().map(|vector| EmbeddedQuery {
            namespace: o.namespace,
            vector,
        })
    }))
}
