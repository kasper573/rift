/// A realm role the game recognizes. The single source of truth for authorization: keycloak
/// provisioning is generated from here (`kc-roles`), and token roles parse into this enum, so
/// no role ever exists as a bare string elsewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Admin,
    Spectate,
}

/// The keycloak groups provisioned from source: a group name and the realm roles it grants.
pub const GROUPS: &[(&str, &[Role])] = &[("admin", &Role::ALL), ("spectator", &[Role::Spectate])];

impl Role {
    pub const ALL: [Role; 2] = [Role::Admin, Role::Spectate];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Spectate => "spectate",
        }
    }

    /// The role a token claim names, if the game recognizes it (issuer-internal roles such as
    /// `default-roles-rift` are not the game's concern).
    pub fn parse(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.as_str() == name)
    }
}

/// Renders the `role <name>` / `group <name> = <roles>` lines `docker/keycloak/provision.sh`
/// applies to the realm; `just` writes them to `docker/keycloak/roles.conf` via `kc-roles`.
pub fn provisioning_conf() -> String {
    let mut out = String::new();
    for role in Role::ALL {
        out.push_str(&format!("role {}\n", role.as_str()));
    }
    for (group, roles) in GROUPS {
        let granted: Vec<&str> = roles.iter().map(|role| role.as_str()).collect();
        out.push_str(&format!("group {group} = {}\n", granted.join(" ")));
    }
    out
}
