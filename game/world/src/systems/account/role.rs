use strum::{AsRefStr, EnumString, VariantArray};

#[derive(Clone, Copy, Debug, PartialEq, Eq, VariantArray, AsRefStr, EnumString)]
pub enum Role {
    Admin,
    Spectate,
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
