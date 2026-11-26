#![allow(clippy::unwrap_used)]

mod support;

use std::io::ErrorKind;

use bonk_catalog::{DownloadError, RemoteFile};
use support::*;

const LEN: usize = (1 << 20) + 123;

fn read(remote: &RemoteFile, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0; len];
    remote.read_exact_at(offset, &mut buf)?;
    Ok(buf)
}

#[test]
fn des_morceaux_epars_se_lisent_sans_tout_telecharger() {
    let body = data(LEN);
    let server = Server::start(body.clone(), |_| Reply::Serve);
    let remote = RemoteFile::open(&server.url()).unwrap();
    assert_eq!(remote.len(), LEN as u64);
    let cases: [(usize, usize); 4] = [(0, 16), (32_768, 2048), (262_140, 10), (LEN - 5, 5)];
    for (offset, len) in cases {
        assert_eq!(
            read(&remote, offset as u64, len).unwrap(),
            body[offset..offset + len],
            "{offset}"
        );
    }
    let requests = server.requests();
    assert!(
        requests.iter().all(|r| r.contains("range: bytes=")),
        "{requests:?}"
    );
    assert!(
        requests[1..]
            .iter()
            .all(|r| r.contains(&format!("if-range: {ETAG}"))),
        "{requests:?}"
    );
}

#[test]
fn apres_une_redirection_chaque_morceau_se_demande_au_meme_miroir() {
    let body = data(LEN);
    let server = Server::start(body.clone(), |_| Reply::Redirect);
    let remote = RemoteFile::open(&server.url()).unwrap();
    assert_eq!(read(&remote, 600_000, 16).unwrap(), body[600_000..600_016]);
    let requests = server.requests();
    assert!(requests[0].starts_with("get /image.iso "), "{requests:?}");
    assert!(
        requests[1..]
            .iter()
            .all(|r| r.starts_with("get /miroir.iso ")),
        "{requests:?}"
    );
}

#[test]
fn un_morceau_deja_lu_ne_se_redemande_pas() {
    let server = Server::start(data(LEN), |_| Reply::Serve);
    let remote = RemoteFile::open(&server.url()).unwrap();
    read(&remote, 40_000, 4096).unwrap();
    let before = server.requests().len();
    read(&remote, 40_000, 4096).unwrap();
    read(&remote, 41_000, 100).unwrap();
    assert_eq!(server.requests().len(), before);
}

#[test]
fn au_dela_de_la_fin_rien_n_est_demande() {
    let server = Server::start(data(LEN), |_| Reply::Serve);
    let remote = RemoteFile::open(&server.url()).unwrap();
    let before = server.requests().len();
    let error = read(&remote, LEN as u64 - 2, 4).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnexpectedEof);
    assert_eq!(server.requests().len(), before);
}

#[test]
fn un_serveur_sans_requetes_partielles_est_refuse_a_l_ouverture() {
    let server = Server::start(data(LEN), |_| Reply::IgnoreRange);
    let error = RemoteFile::open(&server.url()).unwrap_err();
    assert!(matches!(error, DownloadError::Inconsistent(_)), "{error}");
    assert!(error.to_string().contains("partial"), "{error}");
}

#[test]
fn une_reponse_entiere_plus_tard_dit_que_le_fichier_a_change() {
    let server = Server::start(data(LEN), |n| match n {
        0 => Reply::Serve,
        _ => Reply::IgnoreRange,
    });
    let remote = RemoteFile::open(&server.url()).unwrap();
    let error = read(&remote, 600_000, 16).unwrap_err();
    assert!(error.to_string().contains("changed"), "{error}");
}

#[test]
fn une_reponse_mal_placee_est_refusee() {
    let server = Server::start(data(LEN), |n| match n {
        0 => Reply::Serve,
        _ => Reply::WrongStart,
    });
    let remote = RemoteFile::open(&server.url()).unwrap();
    assert!(read(&remote, 600_000, 16).is_err());
}

#[test]
fn une_page_absente_se_dit_avec_son_statut() {
    let server = Server::start(Vec::new(), |_| Reply::Status(404));
    let error = RemoteFile::open(&server.url()).unwrap_err();
    assert!(
        matches!(error, DownloadError::Refused { status: 404, .. }),
        "{error}"
    );
}

#[test]
fn seul_https_est_admis_hors_de_cette_machine() {
    let error = RemoteFile::open("http://example.org/image.iso").unwrap_err();
    assert!(matches!(error, DownloadError::Scheme(_)), "{error}");
}
