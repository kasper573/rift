use bevy_ecs::component::Component;

use crate::role::Role;

/// Who a connection authenticated as; sits on the connection's client entity.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
#[component(immutable)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub roles: Vec<Role>,
}

impl Identity {
    pub fn has_role(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }
}
