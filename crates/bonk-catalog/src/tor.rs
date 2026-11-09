use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// The daemon, then Tor Browser. On loopback a slow answer is not a usable Tor.
const PORTS: [u16; 2] = [9050, 9150];
const PATIENCE: Duration = Duration::from_millis(300);

fn first_open(ports: &[u16]) -> Option<SocketAddr> {
    ports
        .iter()
        .map(|port| SocketAddr::from(([127, 0, 0, 1], *port)))
        .find(|address| TcpStream::connect_timeout(address, PATIENCE).is_ok())
}

/// The port alone does not prove it is Tor: the caller offers it, never enables it.
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
