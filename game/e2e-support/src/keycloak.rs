use std::time::Duration;

use serde_json::Value;

pub struct Keycloak {
    base: String,
    realm: String,
    agent: ureq::Agent,
    admin_token: String,
}

impl Keycloak {
    // The stack's reverse proxy uses a local throwaway CA, so verification is off.
    pub fn connect(base: &str, realm: &str, admin: &str, password: &str) -> Keycloak {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(10)))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .disable_verification(true)
                    .build(),
            )
            .build()
            .into();
        let token = token_request(
            &agent,
            &format!("{base}/realms/master/protocol/openid-connect/token"),
            &[
                ("grant_type", "password"),
                ("client_id", "admin-cli"),
                ("username", admin),
                ("password", password),
            ],
        );
        Keycloak {
            base: base.to_owned(),
            realm: realm.to_owned(),
            agent,
            admin_token: token,
        }
    }

    pub fn create_user(&self, username: &str, password: &str, realm_roles: &[&str]) {
        let body = serde_json::json!({
            "username": username,
            "email": format!("{username}@example.test"),
            "emailVerified": true,
            "firstName": "E2e",
            "lastName": "Tester",
            "enabled": true,
            "requiredActions": [],
            "credentials": [{ "type": "password", "value": password, "temporary": false }],
        });
        let response = self
            .agent
            .post(format!("{}/admin/realms/{}/users", self.base, self.realm))
            .header("Authorization", &format!("Bearer {}", self.admin_token))
            .send_json(&body)
            .expect("create user");
        assert!(
            response.status().is_success(),
            "create user failed: {}",
            response.status()
        );

        if !realm_roles.is_empty() {
            let id = self.user_id(username);
            let roles: Vec<Value> = realm_roles
                .iter()
                .map(|name| {
                    let role: Value = self
                        .agent
                        .get(format!(
                            "{}/admin/realms/{}/roles/{name}",
                            self.base, self.realm
                        ))
                        .header("Authorization", &format!("Bearer {}", self.admin_token))
                        .call()
                        .expect("fetch role")
                        .body_mut()
                        .read_json()
                        .expect("role json");
                    serde_json::json!({ "id": role["id"], "name": role["name"] })
                })
                .collect();
            let response = self
                .agent
                .post(format!(
                    "{}/admin/realms/{}/users/{id}/role-mappings/realm",
                    self.base, self.realm
                ))
                .header("Authorization", &format!("Bearer {}", self.admin_token))
                .send_json(&roles)
                .expect("assign roles");
            assert!(
                response.status().is_success(),
                "assign roles failed: {}",
                response.status()
            );
        }
    }

    /// A password-grant access token for the `rift` client (direct access grants are enabled on
    /// the realm), carrying the user's realm roles.
    pub fn password_token(&self, username: &str, password: &str) -> String {
        token_request(
            &self.agent,
            &format!(
                "{}/realms/{}/protocol/openid-connect/token",
                self.base, self.realm
            ),
            &[
                ("grant_type", "password"),
                ("client_id", "rift"),
                ("scope", "openid"),
                ("username", username),
                ("password", password),
            ],
        )
    }

    fn user_id(&self, username: &str) -> String {
        let users: Value = self
            .agent
            .get(format!(
                "{}/admin/realms/{}/users?username={username}&exact=true",
                self.base, self.realm
            ))
            .header("Authorization", &format!("Bearer {}", self.admin_token))
            .call()
            .expect("look up user")
            .body_mut()
            .read_json()
            .expect("users json");
        users[0]["id"].as_str().expect("user id").to_owned()
    }
}

fn token_request(agent: &ureq::Agent, url: &str, form: &[(&str, &str)]) -> String {
    let mut response = agent
        .post(url)
        .send_form(form.iter().copied())
        .unwrap_or_else(|error| panic!("token request to {url} failed: {error}"));
    let body: Value = response.body_mut().read_json().expect("token json");
    body["access_token"]
        .as_str()
        .unwrap_or_else(|| panic!("no access_token in {body}"))
        .to_owned()
}
