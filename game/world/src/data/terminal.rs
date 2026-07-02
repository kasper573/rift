use crate::systems::account::role::Role;
use crate::systems::terminal::Terminal;

crate::table! {
    Global: Terminal {
        title: "Global",
        access: None,
    },
    Admin: Terminal {
        title: "Admin",
        access: Some(Role::Admin),
    },
}
