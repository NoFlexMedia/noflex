// SPDX-License-Identifier: AGPL-3.0-only
//
// What the schema shares with the server about accounts going away. An
// ordinary deletion is done by the time `deleteAccount` answers, but for what
// can't cascade, which the server's purge worker clears once woken. The last
// administrator's takes the server back to first-run setup: the server restarts
// to erase it, with nothing else running, and every request but `serverInfo` is
// refused from the moment it's asked for.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_graphql::{
    ErrorExtensions, Pos, ServerResult, Variables,
    extensions::{Extension, ExtensionContext, ExtensionFactory, NextParseQuery},
    parser::types::{ExecutableDocument, Selection},
};
use tokio::sync::Notify;

/// `extensions.code` of everything refused while the server is erased.
const SERVER_RESETTING: &str = "SERVER_RESETTING";

/// Root fields answered while the server is erased: enough for a client to see
/// it happening.
const ANSWERED_WHILE_RESETTING: [&str; 4] = ["serverInfo", "__typename", "__schema", "__type"];

/// Shared by the schema, which asks for erasures, and the server, which carries
/// them out. Every signal is a stored permit, so one given before anyone waits
/// isn't lost.
#[derive(Debug, Default)]
pub struct Erasure {
    resetting: AtomicBool,
    restart: Notify,
    purge: Notify,
}

impl Erasure {
    /// One already erasing the server, for what answers while it does.
    pub fn resetting() -> Self {
        let erasure = Self::default();
        erasure.resetting.store(true, Ordering::SeqCst);
        erasure
    }

    /// Whether the server is being erased, so that only `serverInfo` answers.
    pub fn is_resetting(&self) -> bool {
        self.resetting.load(Ordering::SeqCst)
    }

    pub(crate) fn set_resetting(&self, resetting: bool) {
        self.resetting.store(resetting, Ordering::SeqCst);
    }

    /// Resolves once an erasure of the server has been recorded and the server
    /// should restart to carry it out.
    pub async fn restart_requested(&self) {
        self.restart.notified().await;
    }

    pub(crate) fn request_restart(&self) {
        self.restart.notify_one();
    }

    /// Resolves once a deleted account has left something to purge.
    pub async fn purge_requested(&self) {
        self.purge.notified().await;
    }

    pub(crate) fn request_purge(&self) {
        self.purge.notify_one();
    }
}

/// Refuses every operation but a look at the server while it's being erased,
/// before anything resolves: a sign-in or an invite redeemed now would make an
/// account the erasure then takes away.
pub(crate) struct ResetGuard(pub(crate) Arc<Erasure>);

impl ExtensionFactory for ResetGuard {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(ResetGuardExtension(self.0.clone()))
    }
}

struct ResetGuardExtension(Arc<Erasure>);

#[async_trait::async_trait]
impl Extension for ResetGuardExtension {
    async fn parse_query(
        &self,
        ctx: &ExtensionContext<'_>,
        query: &str,
        variables: &Variables,
        next: NextParseQuery<'_>,
    ) -> ServerResult<ExecutableDocument> {
        let document = next.run(ctx, query, variables).await?;
        if self.0.is_resetting() && !only_asks_about_the_server(&document) {
            return Err(resetting_refusal().into_server_error(Pos::default()));
        }
        Ok(document)
    }
}

/// A fragment at the root could hide anything, so it doesn't count.
fn only_asks_about_the_server(document: &ExecutableDocument) -> bool {
    document.operations.iter().all(|(_, operation)| {
        operation
            .node
            .selection_set
            .node
            .items
            .iter()
            .all(|selection| match &selection.node {
                Selection::Field(field) => {
                    ANSWERED_WHILE_RESETTING.contains(&field.node.name.node.as_str())
                }
                _ => false,
            })
    })
}

/// The refusal of anything asked of the server while it's erased.
pub(crate) fn resetting_refusal() -> async_graphql::Error {
    async_graphql::Error::new("this server is being reset")
        .extend_with(|_, ext| ext.set("code", SERVER_RESETTING))
}
