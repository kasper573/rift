use bevy_ecs::component::Component;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::core::content::{Content, ContentFile, ContentId, NamedContent, Nearby};
use crate::core::net::auth::TokenClaims;

#[derive(Component, Clone, Debug, PartialEq, Eq)]
#[component(immutable)]
pub struct AccountIdentity {
    pub id: AccountId,
    pub name: String,
    pub roles: Vec<RoleId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountId(pub String);

impl AccountIdentity {
    /// The account `claims` sign in, with the roles they grant it in the game `content` holds.
    pub fn of(content: &Content, claims: TokenClaims) -> AccountIdentity {
        let roles = content
            .table::<RoleDef>()
            .iter()
            .filter(|(_, role)| claims.listed(&role.claim).contains(&role.value))
            .map(|(id, _)| id)
            .collect();
        AccountIdentity {
            name: claims.display_name(),
            id: AccountId(claims.sub),
            roles,
        }
    }

    /// Whether the account may do what only `roles` may: anything, when no role is asked.
    pub fn holds_any(&self, roles: &[RoleId]) -> bool {
        roles.is_empty() || roles.iter().any(|role| self.roles.contains(role))
    }
}

/// A standing some accounts hold, by which a game lets them do what others may not. An account
/// holds it when the access token it signed in with lists `value` in its claim `claim`.
#[derive(JsonSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleDef {
    pub name: String,
    /// Where the token lists what its account may do: a JSON pointer into its claims, to a list
    /// or to words with spaces between them.
    pub claim: String,
    /// What the claim lists for an account that holds the role.
    pub value: String,
}

pub type RoleId = ContentId<RoleDef>;

impl ContentFile for RoleDef {
    const TABLE: &'static str = "role";

    fn fresh(near: &Nearby) -> Result<RoleDef, String> {
        Ok(RoleDef {
            name: near.name(),
            claim: "/roles".to_owned(),
            value: near.name().to_lowercase(),
        })
    }
}

impl NamedContent for RoleDef {
    fn shown_name(&self) -> &str {
        &self.name
    }
}
