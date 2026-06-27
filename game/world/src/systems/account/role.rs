#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Admin,
    Spectate,
}

pub const GROUPS: &[(&str, &[Role])] = &[("admin", &Role::ALL), ("spectator", &[Role::Spectate])];

impl Role {
    pub const ALL: [Role; 2] = [Role::Admin, Role::Spectate];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Spectate => "spectate",
        }
    }

    pub fn parse(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.as_str() == name)
    }
}

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
