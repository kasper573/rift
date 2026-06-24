//! Prints the keycloak provisioning lines for `docker/keycloak/roles.conf`, so the realm's
//! roles and groups have their single source of truth in [`world::systems::account::role`].

fn main() {
    print!("{}", world::systems::account::role::provisioning_conf());
}
