use bevy_ecs::component::Component;

#[derive(Component, Clone, Debug, PartialEq, Eq)]
#[component(immutable)]
pub struct AccountIdentity {
    pub id: AccountId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AccountId(pub String);
