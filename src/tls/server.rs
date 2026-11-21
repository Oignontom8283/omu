//! The certificate the daemon shows when other devices connect to it over HTTPS (a phone that sends its microphone, for
//! example: browsers only give the microphone to a page served securely).
//!
//! Two sources, in this order:
//!
//! * **The user's own**: `server.pem` (the certificate, and its chain if it has one) and `server-key.pem` (its private key) in the
//!   certificates folder. For a certificate from a real authority, or from the user's own.
//! * **One made by the program**: a self-signed certificate for this computer's names and addresses, kept in the program's data
//!   (`tls/`), made again when it is about to expire or when the computer's addresses changed. A browser shows a warning the
//!   first time (nobody vouches for it): the page then shows its fingerprint, to be compared with the one the browser shows.

use crate::{Error, Trust};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

/// A certificate made by the program is valid for this long, in days.
const VALID_DAYS: i64 = 365;
/// ...and is made again when less than this is left.
const RENEW_BEFORE_DAYS: i64 = 30;

/// What the HTTPS server needs, and what the user may want to know about it.
pub struct Identity {
    pub config: Arc<rustls::ServerConfig>,
    /// The SHA-256 of the certificate, as hexadecimal pairs: what a browser shows under "details".
    pub fingerprint: String,
    /// True when the program made the certificate (false: the user's own).
    pub generated: bool,
    /// The names and addresses the certificate is for (empty for the user's own: not read).
    pub names: Vec<String>,
}

/// The names and addresses of this computer that a certificate must cover: what another device may type to reach it.
pub fn local_names(extra: &[IpAddr]) -> Vec<String> {
    let mut names = vec![
        "localhost".to_owned(),
        "127.0.0.1".to_owned(),
        "::1".to_owned(),
    ];
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(host) = std::env::var(var) {
            let host = host.trim().to_owned();
            if !host.is_empty() && !names.contains(&host) {
                names.push(host);
            }
        }
    }
    let mut ips: Vec<IpAddr> = if_addrs::get_if_addrs()
        .map(|all| {
            all.into_iter()
                .map(|i| i.ip())
                .filter(|ip| ip.is_ipv4() && !ip.is_loopback())
                .collect()
        })
        .unwrap_or_default();
    ips.extend(extra.iter().copied());
    ips.sort();
    ips.dedup();
    for ip in ips {
        let ip = ip.to_string();
        if !names.contains(&ip) {
            names.push(ip);
        }
    }
    names
}

fn fingerprint(der: &CertificateDer<'_>) -> String {
    Sha256::digest(der.as_ref())
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn config(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<rustls::ServerConfig>, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::Server(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(|e| {
            Error::Server(format!(
                "the certificate and its key do not go together: {e}"
            ))
        })?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

fn read_pair(
    cert: &Path,
    key: &Path,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), Error> {
    let chain = CertificateDer::pem_file_iter(cert)
        .map_err(|e| Error::Server(format!("cannot read {}: {e}", cert.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Server(format!("{} is not a certificate: {e}", cert.display())))?;
    if chain.is_empty() {
        return Err(Error::Server(format!(
            "{} holds no certificate",
            cert.display()
        )));
    }
    let key = PrivateKeyDer::from_pem_file(key)
        .map_err(|e| Error::Server(format!("{} is not a private key: {e}", key.display())))?;
    Ok((chain, key))
}

/// The certificate to show: the user's own if they put one in the certificates folder, else the one the program makes and keeps.
pub fn server_identity(
    trust: &Trust,
    data_dir: &Path,
    extra: &[IpAddr],
) -> Result<Identity, Error> {
    let own_cert = trust.folder.join("server.pem");
    let own_key = trust.folder.join("server-key.pem");
    if own_cert.is_file() {
        if !own_key.is_file() {
            return Err(Error::Server(format!(
                "{} is there but its private key is not: put it next to it as {}",
                own_cert.display(),
                own_key.display()
            )));
        }
        let (chain, key) = read_pair(&own_cert, &own_key)?;
        let fingerprint = fingerprint(&chain[0]);
        return Ok(Identity {
            config: config(chain, key)?,
            fingerprint,
            generated: false,
            names: vec![],
        });
    }

    let dir = data_dir.join("tls");
    let (cert_path, key_path, names_path) = (
        dir.join("server.pem"),
        dir.join("server-key.pem"),
        dir.join("names.json"),
    );
    let names = local_names(extra);
    // what was made before is kept as long as it is for the same names and not about to expire
    if let Ok((chain, key)) = read_pair(&cert_path, &key_path) {
        let same = std::fs::read_to_string(&names_path)
            .ok()
            .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
            .is_some_and(|saved| saved == names);
        let fresh = x509_parser::parse_x509_certificate(chain[0].as_ref())
            .ok()
            .and_then(|(_, c)| c.validity().time_to_expiration())
            .is_some_and(|left| left.whole_days() > RENEW_BEFORE_DAYS);
        if same && fresh {
            let fingerprint = fingerprint(&chain[0]);
            return Ok(Identity {
                config: config(chain, key)?,
                fingerprint,
                generated: true,
                names,
            });
        }
    }

    let key_pair = rcgen::KeyPair::generate().map_err(|e| Error::Server(e.to_string()))?;
    let mut params = rcgen::CertificateParams::default();
    params.subject_alt_names = names
        .iter()
        .map(|n| match n.parse::<IpAddr>() {
            Ok(ip) => Ok(rcgen::SanType::IpAddress(ip)),
            Err(_) => n.clone().try_into().map(rcgen::SanType::DnsName),
        })
        .collect::<Result<_, _>>()
        .map_err(|e: rcgen::Error| Error::Server(e.to_string()))?;
    // shown by a browser in its warning about the certificate: the product's name, whatever it is called
    params.distinguished_name.push(
        rcgen::DnType::CommonName,
        format!("{} local server", omu_branding::DISPLAY_NAME),
    );
    let now = time::OffsetDateTime::now_utc();
    params.not_before = now - time::Duration::days(1);
    params.not_after = now + time::Duration::days(VALID_DAYS);
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| Error::Server(e.to_string()))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::Server(format!("cannot create {}: {e}", dir.display())))?;
    std::fs::write(&cert_path, cert.pem()).map_err(|e| Error::Server(e.to_string()))?;
    std::fs::write(&key_path, key_pair.serialize_pem())
        .map_err(|e| Error::Server(e.to_string()))?;
    std::fs::write(
        &names_path,
        serde_json::to_string(&names).unwrap_or_default(),
    )
    .map_err(|e| Error::Server(e.to_string()))?;
    let chain = vec![cert.der().clone()];
    let key = PrivateKeyDer::try_from(key_pair.serialize_der())
        .map_err(|e| Error::Server(e.to_string()))?;
    let fingerprint = fingerprint(&chain[0]);
    Ok(Identity {
        config: config(chain, key)?,
        fingerprint,
        generated: true,
        names,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use omu_types::TrustMode;
    use std::io::{Read, Write};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("omu-tls-server-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn trust_in(dir: &Path) -> Trust {
        Trust {
            mode: TrustMode::Own,
            folder: dir.join("certificates"),
        }
    }

    #[test]
    fn a_certificate_is_made_for_this_computer_kept_and_made_again_when_the_addresses_change() {
        let dir = temp_dir("generated");
        let trust = trust_in(&dir);
        let first = server_identity(&trust, &dir, &[]).unwrap();
        assert!(first.generated);
        assert!(
            first.names.iter().any(|n| n == "localhost")
                && first.names.iter().any(|n| n == "127.0.0.1")
        );
        assert_eq!(
            first.fingerprint.split(':').count(),
            32,
            "{}",
            first.fingerprint
        );
        // the same names: the same certificate
        assert_eq!(
            server_identity(&trust, &dir, &[]).unwrap().fingerprint,
            first.fingerprint
        );
        // a new address (a network was joined): a new certificate that covers it
        let other = server_identity(&trust, &dir, &["10.99.88.77".parse().unwrap()]).unwrap();
        assert_ne!(other.fingerprint, first.fingerprint);
        assert!(other.names.iter().any(|n| n == "10.99.88.77"));
    }

    #[test]
    fn the_users_own_certificate_wins_and_a_missing_key_is_said() {
        let dir = temp_dir("own");
        let trust = trust_in(&dir);
        std::fs::create_dir_all(&trust.folder).unwrap();
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = rcgen::CertificateParams::new(vec!["mine.example".into()])
            .unwrap()
            .self_signed(&key)
            .unwrap();
        std::fs::write(trust.folder.join("server.pem"), cert.pem()).unwrap();
        let err = server_identity(&trust, &dir, &[])
            .err()
            .expect("no key")
            .to_string();
        assert!(err.contains("server-key.pem"), "{err}");
        std::fs::write(trust.folder.join("server-key.pem"), key.serialize_pem()).unwrap();
        let identity = server_identity(&trust, &dir, &[]).unwrap();
        assert!(!identity.generated);
        assert_eq!(identity.fingerprint, fingerprint(cert.der()));
        assert!(
            !dir.join("tls").exists(),
            "nothing was made: the user's certificate is used"
        );
    }

    #[test]
    fn a_client_that_trusts_the_made_certificate_reaches_the_server_over_https() {
        let dir = temp_dir("handshake");
        let trust = trust_in(&dir);
        let identity = server_identity(&trust, &dir, &[]).unwrap();
        let config = identity.config.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(2).flatten() {
                let Ok(conn) = rustls::ServerConnection::new(config.clone()) else {
                    continue;
                };
                let mut tls = rustls::StreamOwned::new(conn, stream);
                let mut request = [0u8; 1024];
                if tls.read(&mut request).is_ok() {
                    let _ = tls.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                    );
                    let _ = tls.flush();
                }
            }
        });
        let url = format!("https://localhost:{port}/");
        // without the certificate in its folder, the client refuses the server
        assert!(crate::agent(&trust).is_err(), "nothing to trust yet");
        let system = Trust {
            mode: TrustMode::System,
            folder: trust.folder.clone(),
        };
        assert!(crate::agent(&system).unwrap().get(&url).call().is_err());
        // with it, the client is served
        std::fs::create_dir_all(&trust.folder).unwrap();
        std::fs::copy(
            dir.join("tls").join("server.pem"),
            trust.folder.join("omu-server.pem"),
        )
        .unwrap();
        let mut response = crate::agent(&trust).unwrap().get(&url).call().unwrap();
        assert_eq!(response.body_mut().read_to_string().unwrap(), "ok");
    }
}
