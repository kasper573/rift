use bevy_terminal::Terminal;

use crate::systems::account::role;

crate::table! {
    Global: Terminal {
        access: None,
    },
    Admin: Terminal {
        access: Some(role::is_admin),
    },
}
