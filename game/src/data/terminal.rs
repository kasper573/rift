use bevy_terminal::Terminal;

use crate::systems::account::role;

crate::table! {
    #![expose]
    Global: Terminal {
        access: None,
    },
    Admin: Terminal {
        access: Some(role::is_admin),
    },
}
