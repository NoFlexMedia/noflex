// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use async_graphql::{Context, InputObject, Object};
use noflex_core::{TagId, UserId};
use noflex_storage::{Storage, domain::UserRestrictions};

use crate::access;

#[derive(InputObject)]
pub struct UserRestrictionsInput {
    pub allowed_parental_rating_ids: Vec<TagId>,
    pub allow_unrated: bool,
    pub max_stream_bitrate_bps: Option<u64>,
    pub max_concurrent_transcodes: Option<u32>,
}

#[derive(Default)]
pub struct RestrictionsQuery;
#[Object]
impl RestrictionsQuery {
    async fn my_restrictions(&self, ctx: &Context<'_>) -> async_graphql::Result<UserRestrictions> {
        let user = access::user(ctx)?;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        Ok(storage
            .get_user_restrictions(user)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .unwrap_or_else(|| UserRestrictions::unrestricted(user)))
    }
    async fn user_restrictions(
        &self,
        ctx: &Context<'_>,
        user_id: UserId,
    ) -> async_graphql::Result<UserRestrictions> {
        access::require_admin(ctx)?;
        let user = user_id;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        Ok(storage
            .get_user_restrictions(user)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?
            .unwrap_or_else(|| UserRestrictions::unrestricted(user)))
    }
}

#[derive(Default)]
pub struct RestrictionsMutation;
#[Object]
impl RestrictionsMutation {
    async fn set_user_restrictions(
        &self,
        ctx: &Context<'_>,
        user_id: UserId,
        input: UserRestrictionsInput,
    ) -> async_graphql::Result<UserRestrictions> {
        access::require_admin(ctx)?;
        let user = user_id;
        let storage = ctx
            .data::<Arc<dyn Storage>>()
            .map_err(|_| async_graphql::Error::new("storage not configured"))?;
        crate::collections::check_item_count(input.allowed_parental_rating_ids.len())?;
        let mut ratings = input.allowed_parental_rating_ids;
        ratings.sort_by_key(|id| id.into_uuid());
        ratings.dedup();
        let policy = UserRestrictions {
            user_id: user,
            allowed_parental_rating_ids: ratings,
            allow_unrated: input.allow_unrated,
            max_stream_bitrate_bps: input.max_stream_bitrate_bps,
            max_concurrent_transcodes: input.max_concurrent_transcodes,
        };
        storage
            .set_user_restrictions(&policy)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
        Ok(policy)
    }
}
