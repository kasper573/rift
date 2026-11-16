use std::io::Cursor;

use engine::core::update::{self, ArchiveKind, OtherBuildAnswer, ReleaseSource, UpdateFailure};

#[test]
fn a_client_installs_only_what_the_release_key_signed() {
    let archive = b"the game client of the next build".to_vec();
    let (key, signature) = signed(&archive);
    let (other_key, _) = signed(&archive);
    let mut tampered = archive.clone();
    tampered[0] ^= 1;
    for (said, archive, key, kept) in [
        ("signed by the key", &archive, &key, Ok(())),
        (
            "changed after signing",
            &tampered,
            &key,
            Err(UpdateFailure::Unsigned),
        ),
        (
            "signed by another key",
            &archive,
            &other_key,
            Err(UpdateFailure::Unsigned),
        ),
    ] {
        assert_eq!(update::verify(archive, &signature, key), kept, "{said}");
    }
    assert_eq!(
        update::verify(&archive, "not a signature", &key),
        Err(UpdateFailure::Unsigned),
        "a signature that doesn't read"
    );
}

#[test]
fn an_update_takes_only_the_game_client_out_of_its_archive() {
    let client = b"the client".to_vec();
    let other = b"something else".to_vec();
    let found = Ok(client.clone());
    let missing = |name: &str| {
        Err(UpdateFailure::Malformed(format!(
            "the archive holds no {name}"
        )))
    };
    for (kind, files, binary, taken) in [
        (
            ArchiveKind::TarGz,
            vec![("readme.txt", &other), ("game-client", &client)],
            "game-client",
            found.clone(),
        ),
        (
            ArchiveKind::TarGz,
            vec![("nested/game-client", &client)],
            "game-client",
            missing("game-client"),
        ),
        (
            ArchiveKind::Zip,
            vec![("game-client.exe", &client), ("game-server.exe", &other)],
            "game-client.exe",
            found,
        ),
        (
            ArchiveKind::Zip,
            vec![("game-server.exe", &other)],
            "game-client.exe",
            missing("game-client.exe"),
        ),
    ] {
        let archive = archived(kind, &files);
        assert_eq!(update::unpack(&archive, kind, binary), taken, "{files:?}");
    }
}

#[test]
fn a_client_whose_server_runs_another_build_updates_once_when_it_knows_where_and_whose_key() {
    let releases = Some("https://downloads.example/releases/");
    let key = Some("RWQ-the-release-key");
    let source = ReleaseSource {
        releases_url: "https://downloads.example/releases".to_owned(),
        key: "RWQ-the-release-key".to_owned(),
    };
    for (said, releases, key, updated_from, answer) in [
        (
            "both known",
            releases,
            key,
            None,
            OtherBuildAnswer::Update(source.clone()),
        ),
        (
            "no releases address",
            None,
            key,
            None,
            OtherBuildAnswer::ByHand,
        ),
        (
            "built without a key",
            releases,
            None,
            None,
            OtherBuildAnswer::ByHand,
        ),
        (
            "already updated",
            releases,
            key,
            Some("2026.11.14-9c2e1a07"),
            OtherBuildAnswer::Updated {
                from: "2026.11.14-9c2e1a07".to_owned(),
            },
        ),
    ] {
        assert_eq!(
            update::answer(releases, key, updated_from),
            answer,
            "{said}"
        );
    }
    let archive = source.archive_url("2026.11.16-180131e3");
    assert!(
        archive.starts_with("https://downloads.example/releases/2026.11.16-180131e3/game-client-"),
        "{archive}"
    );
}

/// A new key pair's public key, and its signature of `bytes`.
fn signed(bytes: &[u8]) -> (String, String) {
    let pair = minisign::KeyPair::generate_unencrypted_keypair().expect("a key pair");
    let signature =
        minisign::sign(None, &pair.sk, Cursor::new(bytes), None, None).expect("a signature");
    (pair.pk.to_base64(), signature.to_string())
}

fn archived(kind: ArchiveKind, files: &[(&str, &Vec<u8>)]) -> Vec<u8> {
    match kind {
        ArchiveKind::TarGz => {
            let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            let mut tar = tar::Builder::new(gz);
            for (name, bytes) in files {
                let mut header = tar::Header::new_gnu();
                header.set_size(bytes.len() as u64);
                header.set_mode(0o755);
                tar.append_data(&mut header, name, bytes.as_slice())
                    .expect("archived");
            }
            tar.into_inner()
                .and_then(|gz| gz.finish())
                .expect("a tar.gz")
        }
        ArchiveKind::Zip => {
            let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (name, bytes) in files {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .expect("a zip entry");
                std::io::Write::write_all(&mut zip, bytes).expect("written");
            }
            zip.finish().expect("a zip").into_inner()
        }
    }
}
