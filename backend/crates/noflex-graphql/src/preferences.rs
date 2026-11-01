// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::{Context, Json, Object};
use noflex_storage::{Storage, domain::UserPreference};
use serde_json::Value;
use time::OffsetDateTime;

use crate::users::authed_principal;

fn storage<'a>(ctx: &'a Context<'a>) -> async_graphql::Result<&'a Arc<dyn Storage>> {
    ctx.data::<Arc<dyn Storage>>()
        .map_err(|_| async_graphql::Error::new("storage not configured"))
}

fn gql_err(e: impl std::fmt::Display) -> async_graphql::Error {
    async_graphql::Error::new(e.to_string())
}

#[derive(Default)]
pub struct PreferencesQuery;

#[Object]
impl PreferencesQuery {
    async fn preferences(&self, ctx: &Context<'_>) -> async_graphql::Result<Vec<UserPreference>> {
        let principal = authed_principal(ctx)?;
        storage(ctx)?
            .list_user_preferences(principal.user_id)
            .await
            .map_err(gql_err)
    }
}

#[derive(Default)]
pub struct PreferencesMutation;

#[Object]
impl PreferencesMutation {
    async fn set_preference(
        &self,
        ctx: &Context<'_>,
        key: String,
        value: Json<Value>,
    ) -> async_graphql::Result<UserPreference> {
        let principal = authed_principal(ctx)?;
        if key.is_empty() {
            return Err(async_graphql::Error::new("preference key is required"));
        }
        let preference = UserPreference {
            user_id: principal.user_id,
            key,
            value,
            updated_at: OffsetDateTime::now_utc(),
        };
        storage(ctx)?
            .set_user_preference(&preference)
            .await
            .map_err(gql_err)?;
        Ok(preference)
    }

    async fn delete_preference(
        &self,
        ctx: &Context<'_>,
        key: String,
    ) -> async_graphql::Result<bool> {
        let principal = authed_principal(ctx)?;
        if key.is_empty() {
            return Err(async_graphql::Error::new("preference key is required"));
        }
        storage(ctx)?
            .delete_user_preference(principal.user_id, &key)
            .await
            .map_err(gql_err)
    }
}
