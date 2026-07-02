use bevy_ecs::prelude::{Entity, World};
use strum::{AsRefStr, EnumString, VariantArray};

use crate::systems::account::identity::Identity;

#[derive(Clone, Copy, Debug, PartialEq, Eq, VariantArray, AsRefStr, EnumString)]
pub enum Role {
    Admin,
    Spectate,
}

pub fn is_admin(world: &World, conn: Entity) -> bool {
    world
        .get::<Identity>(conn)
        .is_some_and(|identity| identity.has_role(Role::Admin))
}

pub const GROUPS: &[(&str, &[Role])] =
    &[("admin", Role::VARIANTS), ("spectator", &[Role::Spectate])];

pub fn provisioning_conf() -> String {
    let mut out = String::new();
    for role in Role::VARIANTS {
        out.push_str(&format!("role {}\n", role.as_ref()));
    }
    for (group, roles) in GROUPS {
        let granted: Vec<&str> = roles.iter().map(|role| role.as_ref()).collect();
        out.push_str(&format!("group {group} = {}\n", granted.join(" ")));
    }
    out
}
