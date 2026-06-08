use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{SystemTime, UNIX_EPOCH};

use auth::Verifier;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

// A throwaway RSA keypair generated for these tests; it protects nothing.
const TEST_KEY_PEM: &str = "-----BEGIN PRIVATE KEY-----
MIIEvAIBADANBgkqhkiG9w0BAQEFAASCBKYwggSiAgEAAoIBAQCLa/LwAeX+rSlW
vMDrFri6gG8xrz6PlimaKYGEtPdmXp0dyqTkwmCmEW0oI61yHohzzSv65f5mpxi4
wu5Sni8aMv5Zcw2yiDuAuoMZr8ePBhxrYmBzBKo2nmp3xrgEVruFm38PwQauQQH5
sJHoAaoFCRwVkf/zyku6po5Lvg1+OPr1VUAI4ORAj9vN+qptIgsziEM2ez96qTjH
Qpnr9fMP0yfm4QeAGHfs4mpF7tY7bo0MIWJldrzc60Ov+UnpprPjohDsozLM4GNJ
pCBqlf3sysrhWztKjVvn2wBn3GNcMM+tLHK4YK09JJQze+5VqoF8P8lZLa4mYRd0
iostZ+P5AgMBAAECggEAAlbBd0s9mY7mqYqaFfxGUVNhGsBA/ST1gfdlNlao2H+q
F4AEzOqZfkkwhFupwK+NJWMkQA0sdwiAeVqjW7jmUtT9F66UFCd8D+j7we7osoxV
dKSu8PmvtPJ0GDs56kf6RvPekbh6MzH4sVhkYjjCHD9Boo8TXTN1ayPvv5RQrsfV
xnt9o0C7kk1FdAxK6P0eFUOX7owYR3qXln/LfHK+suhvXUKmkhJx6qsQOI4FfDOy
tPS8CWNpzUEiYKP7lZWzDGWYyh4hBX3ec/AKWx7sLa5cfd/OiqMSTT5s7s8UGW7t
SxolzZ1mP6jiKUj96WlhQ7Ao+Z+QbmqmavrOm9N+PwKBgQC9YgQqvYyi7otACmGj
Do+h4JY9oLdqwKZmFdKQEXLkRl5gcr6dlRRmPg+5sMV511L9xgYk8aEjDUlLGJ0t
8bRam86FWBbj/Kqg/ySf7KBwKreXdH07+7u39L6HGtYKmLh82xUrlbH06oPbGl4F
MUY5ZB/szTYuMBozqzivMdzo/wKBgQC8dulyNXi06t47g7fnaG5uuFTdF7W+xjNr
zwloRwsVtxpuRkXNkY4iP8n5opsmZU6NoBKvzNDaJoAMQ2p1glFeEx3wrV/DNamo
Qcj3YlhKIVTCYxL20VNt2MAuczjnqBqqX6jTe2/cbkWFwkI+zjizxEKrPNCfeXFA
MV438JV7BwKBgDDT0aE3Z8gmWq6zPoMs4OlqnzHaew/CBeTyIWzVotqqLfEOBIla
g3zs6V8F7ZRBaPtXEAR8bAA+j7QV74iF9esamr+Ue6piXZfO0KGO/7qLuPQKq7NI
bxi5uFnbGG54+6/tSGMJYG11/XMDNFSAZMutPfHu4tY7vrWtoprA72T/AoGAFyV/
nGBG1+l0q9iMkKY50e1fttu/nZOYIyiFXkJDcUJQw7RrxEiZLUmqU7eN2JRepnQ/
d0nvaKuL1HW/MHl15tjwN2wDs+T2VkzmEsQIVepsD4e9f4TL+1TAnbPXDFSQGdav
1HF3lpoQfdIS8sW/Hwz+pytL8BopN5oYmUQ6B6cCgYB2tpcwAvtMXFRyp5sBADas
Eo+ekmsYwyU6so9JyHbpu7K0MgpLPxZzOYQmf2oDDJZHRbUnnt1FlSr1PGyN92We
m3nIjpKZsHXFLm7KAVVTtatrcMcIm37t6D2agGZ3KUX25wokuPtQnc1j7NCeY8Pa
mqYhto8p0xXRMXr98VUzQw==
-----END PRIVATE KEY-----";

const TEST_KEY_N: &str = "i2vy8AHl_q0pVrzA6xa4uoBvMa8-j5YpmimBhLT3Zl6dHcqk5MJgphFtKCOtch6Ic80r-uX-ZqcYuMLuUp4vGjL-WXMNsog7gLqDGa_HjwYca2JgcwSqNp5qd8a4BFa7hZt_D8EGrkEB-bCR6AGqBQkcFZH_88pLuqaOS74Nfjj69VVACODkQI_bzfqqbSILM4hDNns_eqk4x0KZ6_XzD9Mn5uEHgBh37OJqRe7WO26NDCFiZXa83OtDr_lJ6aaz46IQ7KMyzOBjSaQgapX97MrK4Vs7So1b59sAZ9xjXDDPrSxyuGCtPSSUM3vuVaqBfD_JWS2uJmEXdIqLLWfj-Q";
const TEST_KID: &str = "test-key";
const ISSUER: &str = "https://auth.example.test/realms/mp";
const AUDIENCE: &str = "mp";

/// Serve the JWK set over HTTP for a fixed number of requests; returns the JWKS URI.
fn serve_jwks(requests: usize) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let body = format!(
        r#"{{"keys":[{{"kty":"RSA","kid":"{TEST_KID}","alg":"RS256","use":"sig","n":"{TEST_KEY_N}","e":"AQAB"}}]}}"#
    );
    std::thread::spawn(move || {
        for _ in 0..requests {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut scratch = [0u8; 4096];
            let _ = stream.read(&mut scratch);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://127.0.0.1:{port}/realms/mp/protocol/openid-connect/certs")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("epoch")
        .as_secs()
}

fn sign_token(claims: serde_json::Value) -> String {
    let key = EncodingKey::from_rsa_pem(TEST_KEY_PEM.as_bytes()).expect("test key parses");
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(TEST_KID.to_owned());
    encode(&header, &claims, &key).expect("token signs")
}

fn valid_claims() -> serde_json::Value {
    serde_json::json!({
        "iss": ISSUER,
        "azp": AUDIENCE,
        "sub": "user-123",
        "preferred_username": "kasper",
        "realm_access": { "roles": ["spectate", "offline_access"] },
        "exp": now() + 300,
    })
}

#[test]
fn accepts_a_valid_token() {
    let jwks_uri = serve_jwks(1);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, false);
    let claims = verifier.verify(&sign_token(valid_claims())).expect("valid");
    assert_eq!(claims.subject, "user-123");
    assert_eq!(claims.name, "kasper");
    assert_eq!(claims.roles, vec!["spectate", "offline_access"]);
}

#[test]
fn token_without_realm_roles_has_none() {
    let jwks_uri = serve_jwks(1);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, false);
    let mut claims = valid_claims();
    claims
        .as_object_mut()
        .expect("object")
        .remove("realm_access");
    let claims = verifier.verify(&sign_token(claims)).expect("valid");
    assert!(claims.roles.is_empty());
}

#[test]
fn rejects_wrong_issuer() {
    let jwks_uri = serve_jwks(1);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, false);
    let mut claims = valid_claims();
    claims["iss"] = "https://evil.example/realms/mp".into();
    assert!(verifier.verify(&sign_token(claims)).is_err());
}

#[test]
fn rejects_wrong_authorized_party() {
    let jwks_uri = serve_jwks(1);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, false);
    let mut claims = valid_claims();
    claims["azp"] = "other-app".into();
    assert!(verifier.verify(&sign_token(claims)).is_err());
}

#[test]
fn rejects_expired_token() {
    let jwks_uri = serve_jwks(1);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, false);
    let mut claims = valid_claims();
    claims["exp"] = (now() - 3600).into();
    assert!(verifier.verify(&sign_token(claims)).is_err());
}

#[test]
fn rejects_tampered_token() {
    let jwks_uri = serve_jwks(1);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, false);
    let token = sign_token(valid_claims());
    // Flip a character in the payload section.
    let mut parts: Vec<String> = token.split('.').map(str::to_owned).collect();
    let mut payload = parts[1].clone().into_bytes();
    payload[0] = if payload[0] == b'A' { b'B' } else { b'A' };
    parts[1] = String::from_utf8(payload).expect("ascii");
    assert!(verifier.verify(&parts.join(".")).is_err());
}

#[test]
fn rejects_garbage_tokens() {
    let jwks_uri = serve_jwks(1);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, false);
    assert!(verifier.verify("").is_err());
    assert!(verifier.verify("not-a-jwt").is_err());
    assert!(verifier.verify("bypass:kasper").is_err());
}

#[test]
fn bypass_tokens_when_enabled() {
    let jwks_uri = serve_jwks(0);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, true);
    let claims = verifier.verify("bypass:kasper").expect("bypass allowed");
    assert_eq!(claims.subject, "bypass:kasper");
    assert_eq!(claims.name, "kasper");
    assert!(claims.roles.is_empty(), "bypass users hold no roles");
}

#[test]
fn warm_prefetches_the_key_set() {
    let jwks_uri = serve_jwks(1);
    let mut verifier = Verifier::new(ISSUER, AUDIENCE, &jwks_uri, false);
    verifier.warm().expect("jwks reachable");
    // The single served request is spent; verification must succeed from the cached set.
    let claims = verifier.verify(&sign_token(valid_claims())).expect("valid");
    assert_eq!(claims.subject, "user-123");
}
