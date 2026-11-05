// SPDX-License-Identifier: AGPL-3.0-only
//
// Embeddings from an OpenAI-compatible API an administrator configured in place
// of the local embedder. The one egress the host performs for itself rather than
// through a plugin's allowlist: the URL comes from the administrator, not from a
// plugin, and nothing reaches it unless they ask.

use std::time::Duration;

use noflex_storage::domain::EmbeddingEndpoint;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{EmbedDocument, EmbedInput, manifest::EmbeddingClaim};

/// Long enough for a batch on a loaded server; a search only waits for
/// [`QUERY_TIMEOUT`].
const DOCUMENT_TIMEOUT: Duration = Duration::from_secs(120);

/// Hybrid search falls back to lexical rather than keep someone waiting.
const QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// Attempts at a document batch before leaving it for the next sweep. Queries
/// get one: their fallback is immediate.
const DOCUMENT_ATTEMPTS: u32 = 4;

/// The longest wait a rate limit's `Retry-After` gets to impose per attempt.
const MAX_RETRY_DELAY: Duration = Duration::from_secs(60);

/// Why a call to the endpoint failed. Messages are safe to show an administrator:
/// they hold the provider's explanation but never the key or the URL's query.
#[derive(Debug, Error)]
pub enum RemoteEmbedError {
    #[error("could not reach the embeddings API: {0}")]
    Unreachable(String),
    #[error("the embeddings API answered {status}: {message}")]
    Rejected { status: u16, message: String },
    #[error("the embeddings API answered with something unusable: {0}")]
    Malformed(String),
}

/// A configured endpoint, ready to call.
#[derive(Debug)]
pub struct RemoteEmbedder {
    endpoint: EmbeddingEndpoint,
    claim: EmbeddingClaim,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct EmbeddingsResponse {
    data: Vec<EmbeddingRow>,
}

#[derive(Deserialize)]
struct EmbeddingRow {
    index: usize,
    embedding: Vec<f32>,
}

impl RemoteEmbedder {
    /// Prepares calls to `endpoint` without making one.
    ///
    /// # Errors
    /// [`RemoteEmbedError::Unreachable`] if no HTTP client can be built.
    pub fn new(endpoint: EmbeddingEndpoint) -> Result<Self, RemoteEmbedError> {
        // Without redirects, the key can only reach the host the administrator named.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| RemoteEmbedError::Unreachable(e.without_url().to_string()))?;
        Ok(Self {
            claim: claim_for(&endpoint),
            endpoint,
            client,
        })
    }

    /// Measures what `endpoint` returns by asking it for one vector, proving the
    /// URL, key and model work on the way, and returns it with `dimensions` set.
    ///
    /// # Errors
    /// Whatever the call fails with, for showing to whoever configured it.
    pub async fn probe(
        mut endpoint: EmbeddingEndpoint,
    ) -> Result<EmbeddingEndpoint, RemoteEmbedError> {
        let vectors = Self::new(endpoint.clone())?
            .request(vec!["test".to_string()], QUERY_TIMEOUT)
            .await?;
        let dimensions = vectors.first().map_or(0, Vec::len);
        if dimensions == 0 {
            return Err(RemoteEmbedError::Malformed("an empty vector".to_string()));
        }
        endpoint.dimensions = u32::try_from(dimensions)
            .map_err(|_| RemoteEmbedError::Malformed("an enormous vector".to_string()))?;
        Ok(endpoint)
    }

    /// What this endpoint's vectors are stored and searched under.
    pub fn claim(&self) -> &EmbeddingClaim {
        &self.claim
    }

    /// The host named by the endpoint's URL, for display.
    pub fn host(&self) -> Option<String> {
        let url = url::Url::parse(&self.endpoint.url).ok()?;
        let host = url.host_str()?;
        Some(match url.port() {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        })
    }

    /// One unit vector per input, in input order, each of the claimed
    /// dimensionality. Document batches retry rate limits and server errors.
    ///
    /// # Errors
    /// The last failure, or [`RemoteEmbedError::Malformed`] when the answer doesn't
    /// match the claim, which means the model behind the URL has changed.
    pub async fn embed(&self, inputs: &[EmbedInput]) -> Result<Vec<Vec<f32>>, RemoteEmbedError> {
        let is_query = matches!(inputs, [EmbedInput::Query(_)]);
        let texts = inputs
            .iter()
            .map(|input| self.text(input))
            .collect::<Vec<_>>();
        let vectors = if is_query {
            self.request(texts, QUERY_TIMEOUT).await?
        } else {
            self.request_with_retries(texts).await?
        };
        let want = self.claim.dimensions as usize;
        if let Some(vector) = vectors.iter().find(|vector| vector.len() != want) {
            return Err(RemoteEmbedError::Malformed(format!(
                "{} dimensions where {want} were measured; save the endpoint again",
                vector.len()
            )));
        }
        Ok(vectors)
    }

    fn text(&self, input: &EmbedInput) -> String {
        match input {
            EmbedInput::Query(query) => format!("{}{query}", self.endpoint.query_prefix),
            EmbedInput::Document(EmbedDocument { title, text }) if text.is_empty() => {
                format!("{}{title}", self.endpoint.document_prefix)
            }
            EmbedInput::Document(EmbedDocument { title, text }) => {
                format!("{}{title}. {text}", self.endpoint.document_prefix)
            }
        }
    }

    async fn request_with_retries(
        &self,
        texts: Vec<String>,
    ) -> Result<Vec<Vec<f32>>, RemoteEmbedError> {
        let mut attempt = 1;
        loop {
            let error = match self.post(&texts, DOCUMENT_TIMEOUT).await {
                Ok(vectors) => return Ok(vectors),
                Err(failure) => failure,
            };
            if attempt >= DOCUMENT_ATTEMPTS || !error.retryable {
                return Err(error.error);
            }
            let delay = error
                .retry_after
                .unwrap_or(Duration::from_secs(1 << attempt))
                .min(MAX_RETRY_DELAY);
            tracing::debug!(attempt, ?delay, error = %error.error, "embeddings API: retrying");
            tokio::time::sleep(delay).await;
            attempt += 1;
        }
    }

    async fn request(
        &self,
        texts: Vec<String>,
        timeout: Duration,
    ) -> Result<Vec<Vec<f32>>, RemoteEmbedError> {
        self.post(&texts, timeout)
            .await
            .map_err(|failure| failure.error)
    }

    async fn post(&self, texts: &[String], timeout: Duration) -> Result<Vec<Vec<f32>>, Failure> {
        let mut body = serde_json::json!({
            "model": self.endpoint.model,
            "input": texts,
            "encoding_format": "float",
        });
        if let Some(dimensions) = self.endpoint.request_dimensions {
            body["dimensions"] = dimensions.into();
        }
        let mut request = self
            .client
            .post(format!("{}/embeddings", base_url(&self.endpoint.url)))
            .timeout(timeout)
            .json(&body);
        if let Some(key) = &self.endpoint.api_key {
            request = request.bearer_auth(key);
        }
        let response = request.send().await.map_err(|e| Failure {
            error: RemoteEmbedError::Unreachable(e.without_url().to_string()),
            retryable: true,
            retry_after: None,
        })?;
        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|value| value.to_str().ok()?.parse().ok())
                .map(Duration::from_secs);
            let message = provider_message(&response.text().await.unwrap_or_default());
            return Err(Failure {
                error: RemoteEmbedError::Rejected {
                    status: status.as_u16(),
                    message,
                },
                retryable: status.as_u16() == 429 || status.is_server_error(),
                retry_after,
            });
        }
        let malformed = |message: String| Failure {
            error: RemoteEmbedError::Malformed(message),
            retryable: false,
            retry_after: None,
        };
        let answer = response
            .json::<EmbeddingsResponse>()
            .await
            .map_err(|e| malformed(e.without_url().to_string()))?;
        ordered_unit_vectors(answer, texts.len()).map_err(malformed)
    }
}

struct Failure {
    error: RemoteEmbedError,
    retryable: bool,
    retry_after: Option<Duration>,
}

/// The base with what people commonly paste beyond it removed, so that it joins
/// cleanly with `/embeddings`.
fn base_url(url: &str) -> &str {
    let url = url.trim_end_matches('/');
    url.strip_suffix("/embeddings").unwrap_or(url)
}

/// Everything that changes the vectors an endpoint produces goes into its
/// namespace, so changing any of it leaves the old vectors behind and reindexes.
/// The key doesn't: rotating one shouldn't cost a reindex. Neither does a trailing
/// slash.
fn claim_for(endpoint: &EmbeddingEndpoint) -> EmbeddingClaim {
    let mut hash = Sha256::new();
    for part in [
        base_url(&endpoint.url),
        &endpoint.model,
        &endpoint.query_prefix,
        &endpoint.document_prefix,
    ] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    hash.update(endpoint.dimensions.to_le_bytes());
    let digest = hash.finalize();
    let id = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    EmbeddingClaim {
        namespace: format!("remote-{id}"),
        model_id: endpoint.model.clone(),
        dimensions: endpoint.dimensions,
    }
}

/// OpenAI's error shape where a provider uses it, otherwise the start of the body.
fn provider_message(body: &str) -> String {
    let parsed = serde_json::from_str::<serde_json::Value>(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|value| {
            value["error"]["message"]
                .as_str()
                .or(value["error"].as_str())
        })
        .unwrap_or(body);
    message.chars().take(200).collect()
}

/// Search compares by distance, which only ranks by similarity between unit
/// vectors, and not every server returns those.
fn ordered_unit_vectors(
    mut answer: EmbeddingsResponse,
    expected: usize,
) -> Result<Vec<Vec<f32>>, String> {
    if answer.data.len() != expected {
        return Err(format!(
            "{} vectors for {expected} texts",
            answer.data.len()
        ));
    }
    answer.data.sort_by_key(|row| row.index);
    if answer
        .data
        .iter()
        .enumerate()
        .any(|(position, row)| row.index != position)
    {
        return Err("vectors with duplicate or missing indexes".to_string());
    }
    Ok(answer
        .data
        .into_iter()
        .map(|row| {
            let mut vector = row.embedding;
            let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
            if norm > 0.0 {
                vector.iter_mut().for_each(|value| *value /= norm);
            }
            vector
        })
        .collect())
}
