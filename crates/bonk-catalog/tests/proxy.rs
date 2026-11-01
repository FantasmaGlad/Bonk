//! Le proxy : le nom passe par lui, et sans lui rien ne part. Fichier à part : le réglage vaut pour tout le
//! processus, et chaque fichier de test en a un.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::Duration;

/// Serveur SOCKS5 minimal : note le nom demandé, répond une page.
fn faux_socks(serveur: TcpListener, noms: mpsc::Sender<String>) {
    std::thread::spawn(move || {
        while let Ok((mut flux, _)) = serveur.accept() {
            let _ = echange(&mut flux, &noms);
        }
    });
}

fn echange(flux: &mut TcpStream, noms: &mpsc::Sender<String>) -> std::io::Result<()> {
    let mut entete = [0u8; 2];
    flux.read_exact(&mut entete)?;
    let mut methodes = vec![0u8; usize::from(entete[1])];
    flux.read_exact(&mut methodes)?;
    flux.write_all(&[5, 0])?;
    let mut demande = [0u8; 4];
    flux.read_exact(&mut demande)?;
    // Type 3 : un nom, que seul le proxy résout. Tout autre type veut dire que le DNS est parti en direct.
    if demande[3] != 3 {
        let _ = noms.send(format!("adresse de type {}", demande[3]));
        return Ok(());
    }
    let mut longueur = [0u8; 1];
    flux.read_exact(&mut longueur)?;
    let mut nom = vec![0u8; usize::from(longueur[0])];
    flux.read_exact(&mut nom)?;
    let mut port = [0u8; 2];
    flux.read_exact(&mut port)?;
    let _ = noms.send(String::from_utf8_lossy(&nom).into_owned());
    flux.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])?;
    let mut requete = [0u8; 1024];
    let _ = flux.read(&mut requete)?;
    flux.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
}

#[test]
fn le_nom_passe_par_le_proxy_et_sans_proxy_rien_ne_part() -> Result<(), Box<dyn std::error::Error>>
{
    assert!(
        bonk_catalog::set_proxy("socks5://127.0.0.1:1").is_err(),
        "le DNS partirait en direct"
    );
    assert!(bonk_catalog::set_proxy("http://127.0.0.1:1").is_err());

    let socks = TcpListener::bind("127.0.0.1:0")?;
    let port = socks.local_addr()?.port();
    let (noms, recus) = mpsc::channel();
    faux_socks(socks, noms);
    bonk_catalog::set_proxy(&format!("socks5h://127.0.0.1:{port}"))?;
    let page = bonk_catalog::fetch_text("http://localhost:9/page")?;
    assert_eq!(page, "ok");
    assert_eq!(recus.recv_timeout(Duration::from_secs(5))?, "localhost");

    // Proxy éteint : une erreur, jamais un repli vers le serveur demandé, qui écoute pourtant.
    let ferme = TcpListener::bind("127.0.0.1:0")?;
    let port_ferme = ferme.local_addr()?.port();
    drop(ferme);
    let direct = TcpListener::bind("127.0.0.1:0")?;
    direct.set_nonblocking(true)?;
    let adresse = format!("http://localhost:{}/page", direct.local_addr()?.port());
    bonk_catalog::set_proxy(&format!("socks5h://127.0.0.1:{port_ferme}"))?;
    assert!(bonk_catalog::fetch_text(&adresse).is_err());
    assert!(
        direct.accept().is_err(),
        "connexion directe malgré le proxy"
    );
    Ok(())
}
