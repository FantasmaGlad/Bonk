//! Tor déjà lancé sur cette machine : Bonk le signale et ne s'en sert que si on le lui demande.

use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Le démon, puis Tor Browser. Sur la boucle locale, une réponse lente n'est pas un Tor utilisable.
const PORTS: [u16; 2] = [9050, 9150];
const PATIENCE: Duration = Duration::from_millis(300);

fn first_open(ports: &[u16]) -> Option<SocketAddr> {
    ports
        .iter()
        .map(|port| SocketAddr::from(([127, 0, 0, 1], *port)))
        .find(|address| TcpStream::connect_timeout(address, PATIENCE).is_ok())
}

/// L'adresse d'un proxy qui écoute sur un port de Tor. Le port seul ne prouve pas que c'est Tor :
/// l'appelant propose, sans jamais activer.
#[must_use]
pub fn detect() -> Option<String> {
    first_open(&PORTS).map(|address| format!("socks5h://{address}"))
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn trouve_le_premier_port_qui_ecoute_et_rien_sinon() -> std::io::Result<()> {
        let ecoute = TcpListener::bind("127.0.0.1:0")?;
        let ouvert = ecoute.local_addr()?.port();
        let libre = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        assert_eq!(first_open(&[libre, ouvert]).map(|a| a.port()), Some(ouvert));
        assert_eq!(first_open(&[libre]), None);
        Ok(())
    }
}
