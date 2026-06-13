//! Prints the keycloak provisioning lines for `docker/keycloak/roles.conf`, so the realm's
//! roles and groups have their single source of truth in [`world::role`].

fn main() {
    print!("{}", world::role::provisioning_conf());
}
