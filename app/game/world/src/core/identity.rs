use bevy_ecs::component::Component;

/// Who a connection authenticated as; sits on the connection's client entity.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
#[component(immutable)]
pub struct Identity {
    pub id: String,
    pub name: String,
    pub roles: Vec<String>,
}

impl Identity {
    pub fn has_role(&self, role: &str) -> bool {
        self.roles.iter().any(|held| held == role)
    }
}
