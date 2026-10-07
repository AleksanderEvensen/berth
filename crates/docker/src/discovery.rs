//! Synchronous Docker endpoint discovery. Run socket probes off the UI thread.

use std::{fs, io, path::PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{ConnectError, Endpoint, TlsConfig};

/// Inputs to discovery. Only `from_process` reads process environment variables.
#[derive(Debug, Clone)]
pub struct DiscoveryEnv {
    pub docker_host: Option<String>,
    pub docker_context: Option<String>,
    pub docker_config_dir: Option<PathBuf>,
    pub docker_tls_verify: Option<String>,
    pub docker_cert_path: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub xdg_runtime_dir: Option<PathBuf>,
    /// Defaults to `/var/run/docker.sock`; overridable for isolated tests.
    pub rootful_socket: PathBuf,
}

impl Default for DiscoveryEnv {
    fn default() -> Self {
        Self {
            docker_host: None,
            docker_context: None,
            docker_config_dir: None,
            docker_tls_verify: None,
            docker_cert_path: None,
            home: None,
            xdg_runtime_dir: None,
            rootful_socket: PathBuf::from("/var/run/docker.sock"),
        }
    }
}

impl DiscoveryEnv {
    #[must_use]
    pub fn from_process() -> Self {
        fn string(key: &str) -> Option<String> {
            std::env::var(key).ok().filter(|s| !s.is_empty())
        }
        fn path(key: &str) -> Option<PathBuf> {
            std::env::var_os(key)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        }
        Self {
            docker_host: string("DOCKER_HOST"),
            docker_context: string("DOCKER_CONTEXT"),
            docker_config_dir: path("DOCKER_CONFIG"),
            docker_tls_verify: string("DOCKER_TLS_VERIFY"),
            docker_cert_path: path("DOCKER_CERT_PATH"),
            home: path("HOME").or_else(|| path("USERPROFILE")),
            xdg_runtime_dir: path("XDG_RUNTIME_DIR"),
            ..Self::default()
        }
    }

    fn config_dir(&self) -> Option<PathBuf> {
        self.docker_config_dir
            .clone()
            .or_else(|| self.home.as_ref().map(|h| h.join(".docker")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndpointSource {
    DockerHost,
    Context(String),
    Default(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    pub endpoint: Endpoint,
    pub source: EndpointSource,
}

#[derive(Deserialize)]
struct DockerConfig {
    #[serde(rename = "currentContext")]
    current_context: Option<String>,
}

#[derive(Deserialize)]
struct ContextMeta {
    #[serde(rename = "Endpoints")]
    endpoints: ContextEndpoints,
}

#[derive(Deserialize)]
struct ContextName {
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Deserialize)]
struct ContextEndpoints {
    docker: ContextDocker,
}

#[derive(Deserialize)]
struct ContextDocker {
    #[serde(rename = "Host")]
    host: String,
    #[serde(rename = "SkipTLSVerify", default)]
    skip_tls_verify: bool,
}

fn invalid(message: impl Into<String>) -> ConnectError {
    ConnectError::InvalidConfig(message.into())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<T, ConnectError> {
    let data = fs::read(path).map_err(|e| invalid(format!("{}: {e}", path.display())))?;
    serde_json::from_slice(&data).map_err(|e| invalid(format!("{}: {e}", path.display())))
}

/// Parse a `DOCKER_HOST`-style address without touching the network or disk.
///
/// # Errors
/// Returns [`ConnectError::InvalidConfig`] for unsupported schemes, relative
/// socket paths, missing addresses or embedded whitespace.
pub fn parse_host(host: &str) -> Result<Endpoint, ConnectError> {
    let (scheme, address) = host
        .split_once("://")
        .ok_or_else(|| invalid(format!("invalid Docker host: {host}")))?;
    if address.is_empty() || host.chars().any(char::is_whitespace) {
        return Err(invalid(format!("invalid Docker host: {host}")));
    }
    match scheme {
        "unix" if address.starts_with('/') => Ok(Endpoint::Unix(address.into())),
        "npipe" => Ok(Endpoint::NamedPipe(address.into())),
        "tcp" | "http" | "https" if !address.starts_with('/') => Ok(Endpoint::Tcp {
            url: host.into(),
            tls: None,
        }),
        _ => Err(invalid(format!(
            "unsupported or invalid Docker host: {host}"
        ))),
    }
}

/// Resolve explicit settings first, then probe platform socket fallbacks in order.
/// Explicit settings are never probed or silently replaced by a fallback.
///
/// # Errors
/// Returns an error for invalid explicit settings, unreadable context files,
/// or when no accessible default endpoint is available.
pub fn discover(env: &DiscoveryEnv) -> Result<Discovered, ConnectError> {
    if let Some(host) = env.docker_host.as_deref().filter(|s| !s.is_empty()) {
        let mut endpoint = parse_host(host)?;
        if let Endpoint::Tcp { tls, .. } = &mut endpoint
            && (host.starts_with("https://")
                || env
                    .docker_tls_verify
                    .as_ref()
                    .is_some_and(|s| !s.is_empty()))
        {
            let cert_dir = env
                .docker_cert_path
                .clone()
                .or_else(|| env.home.as_ref().map(|h| h.join(".docker")))
                .ok_or_else(|| invalid("TLS requires DOCKER_CERT_PATH or a home directory"))?;
            *tls = Some(TlsConfig {
                cert_dir,
                verify: true,
            });
        }
        return Ok(Discovered {
            endpoint,
            source: EndpointSource::DockerHost,
        });
    }

    let config_dir = env.config_dir();
    let context = match env.docker_context.as_ref().filter(|s| !s.is_empty()) {
        Some(name) => Some(name.clone()),
        None => match config_dir.as_ref() {
            Some(dir) => {
                let path = dir.join("config.json");
                match fs::read(&path) {
                    Ok(data) => {
                        let config: DockerConfig = serde_json::from_slice(&data)
                            .map_err(|e| invalid(format!("{}: {e}", path.display())))?;
                        config.current_context
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                    Err(e) => return Err(invalid(format!("{}: {e}", path.display()))),
                }
            }
            None => None,
        },
    };
    if let Some(name) = context.filter(|s| !s.is_empty() && s != "default") {
        let dir = config_dir
            .ok_or_else(|| invalid("Docker context requires DOCKER_CONFIG or a home directory"))?;
        let hash = format!("{:x}", Sha256::digest(name.as_bytes()));
        let meta: ContextMeta =
            read_json(&dir.join("contexts/meta").join(&hash).join("meta.json"))?;
        let mut endpoint = parse_host(&meta.endpoints.docker.host)?;
        if let Endpoint::Tcp { tls, url } = &mut endpoint {
            let cert_dir = dir.join("contexts/tls").join(&hash).join("docker");
            if cert_dir.is_dir() || url.starts_with("https://") {
                *tls = Some(TlsConfig {
                    cert_dir,
                    verify: !meta.endpoints.docker.skip_tls_verify,
                });
            }
        }
        return Ok(Discovered {
            endpoint,
            source: EndpointSource::Context(name),
        });
    }

    let mut candidates = vec![(Endpoint::Unix(env.rootful_socket.clone()), "rootful Docker")];
    if let Some(runtime) = &env.xdg_runtime_dir {
        candidates.push((
            Endpoint::Unix(runtime.join("docker.sock")),
            "rootless Docker",
        ));
        candidates.push((Endpoint::Unix(runtime.join("podman/podman.sock")), "Podman"));
    }
    #[cfg(windows)]
    candidates.push((
        Endpoint::NamedPipe("//./pipe/docker_engine".into()),
        "Docker named pipe",
    ));
    #[cfg(target_os = "macos")]
    if let Some(home) = &env.home {
        for (path, label) in [
            (".docker/run/docker.sock", "Docker Desktop"),
            (".orbstack/run/docker.sock", "OrbStack"),
            (".colima/default/docker.sock", "Colima"),
        ] {
            candidates.push((Endpoint::Unix(home.join(path)), label));
        }
    }
    discover_candidates(candidates)
}

/// Names of the Docker contexts under the Docker config directory, sorted.
/// The CLI's implicit `default` context is not stored there, so it is not
/// listed; unreadable entries are skipped.
#[must_use]
pub fn docker_contexts(env: &DiscoveryEnv) -> Vec<String> {
    let Some(dir) = env.config_dir().map(|d| d.join("contexts/meta")) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| read_json::<ContextName>(&entry.path().join("meta.json")).ok())
        .map(|meta| meta.name)
        .filter(|name| !name.is_empty() && name != "default")
        .collect();
    names.sort();
    names.dedup();
    names
}

fn discover_candidates(
    candidates: Vec<(Endpoint, &'static str)>,
) -> Result<Discovered, ConnectError> {
    let mut tried = Vec::new();
    let mut permission_denied = None;
    for (endpoint, label) in candidates {
        tried.push(endpoint.to_string());
        match probe(&endpoint) {
            Ok(()) => {
                return Ok(Discovered {
                    endpoint,
                    source: EndpointSource::Default(label),
                });
            }
            Err(error @ ConnectError::PermissionDenied { .. }) => {
                if permission_denied.is_none() {
                    permission_denied = Some(error);
                }
            }
            Err(_) => {}
        }
    }
    Err(permission_denied.unwrap_or(ConnectError::NotFound { tried }))
}

fn probe_error(endpoint: &Endpoint, error: &io::Error) -> ConnectError {
    let endpoint = endpoint.to_string();
    match error.kind() {
        io::ErrorKind::NotFound => ConnectError::NotFound {
            tried: vec![endpoint],
        },
        io::ErrorKind::PermissionDenied => ConnectError::PermissionDenied { endpoint },
        io::ErrorKind::ConnectionRefused => ConnectError::Refused { endpoint },
        _ => invalid(format!("cannot probe {endpoint}: {error}")),
    }
}

/// Test local transport connectability, without making a Docker API request.
///
/// TCP probing is intentionally unsupported: explicit remote hosts are resolved
/// without connecting, and the engine client owns their connection and TLS.
///
/// # Errors
/// Returns an error if the transport is unsupported or the connection fails.
pub fn probe(endpoint: &Endpoint) -> Result<(), ConnectError> {
    match endpoint {
        #[cfg(unix)]
        Endpoint::Unix(path) => std::os::unix::net::UnixStream::connect(path)
            .map(|_| ())
            .map_err(|e| probe_error(endpoint, &e)),
        #[cfg(windows)]
        Endpoint::NamedPipe(pipe) => fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe)
            .map(|_| ())
            .map_err(|e| probe_error(endpoint, &e)),
        _ => Err(invalid(format!(
            "probing this transport is unsupported: {endpoint}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn env(dir: &TempDir) -> DiscoveryEnv {
        DiscoveryEnv {
            home: Some(dir.path().join("home")),
            docker_config_dir: Some(dir.path().join("config")),
            rootful_socket: dir.path().join("rootful.sock"),
            xdg_runtime_dir: Some(dir.path().join("runtime")),
            ..DiscoveryEnv::default()
        }
    }

    fn write(path: &std::path::Path, data: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    const REMOTE_HASH: &str = "b71199ebd070b36beab7317920c2c2f1d777df8d05e5527d8458fda57cb17a7a";

    fn context(env: &DiscoveryEnv, host: &str, skip: bool) -> PathBuf {
        let dir = env.config_dir().unwrap();
        write(
            &dir.join("contexts/meta")
                .join(REMOTE_HASH)
                .join("meta.json"),
            &serde_json::json!({"Endpoints": {"docker": {"Host": host, "SkipTLSVerify": skip}}})
                .to_string(),
        );
        dir.join("contexts/tls").join(REMOTE_HASH).join("docker")
    }

    #[test]
    fn docker_host_forms_and_precedence() {
        let dir = TempDir::new().unwrap();
        let mut env = env(&dir);
        env.docker_context = Some("missing-context".into());
        for (host, expected) in [
            (
                "unix:///missing.sock",
                Endpoint::Unix("/missing.sock".into()),
            ),
            (
                "npipe:////./pipe/docker_engine",
                Endpoint::NamedPipe("//./pipe/docker_engine".into()),
            ),
            (
                "tcp://localhost:2375",
                Endpoint::Tcp {
                    url: "tcp://localhost:2375".into(),
                    tls: None,
                },
            ),
            (
                "http://localhost:2375",
                Endpoint::Tcp {
                    url: "http://localhost:2375".into(),
                    tls: None,
                },
            ),
            (
                "https://localhost:2376",
                Endpoint::Tcp {
                    url: "https://localhost:2376".into(),
                    tls: Some(TlsConfig {
                        cert_dir: env.home.as_ref().unwrap().join(".docker"),
                        verify: true,
                    }),
                },
            ),
        ] {
            env.docker_host = Some(host.into());
            assert_eq!(
                discover(&env).unwrap(),
                Discovered {
                    endpoint: expected,
                    source: EndpointSource::DockerHost
                }
            );
        }
    }

    #[test]
    fn parse_host_rejects_what_discovery_would() {
        for host in [
            "unix://relative.sock",
            "tcp:///localhost:2375",
            "tcp://local host:2375",
            "ssh://box",
            "tcp://",
            "/var/run/docker.sock",
        ] {
            assert!(parse_host(host).is_err(), "{host}");
        }
        assert!(parse_host("unix:///var/run/docker.sock").is_ok());
    }

    #[test]
    fn lists_named_contexts() {
        let dir = TempDir::new().unwrap();
        let env = env(&dir);
        assert_eq!(docker_contexts(&env), Vec::<String>::new());
        let meta = env.config_dir().unwrap().join("contexts/meta");
        for (hash, name) in [("b", "remote"), ("a", "colima"), ("c", "default")] {
            write(
                &meta.join(hash).join("meta.json"),
                &serde_json::json!({"Name": name, "Endpoints": {}}).to_string(),
            );
        }
        write(&meta.join("d/meta.json"), "{ broken");
        assert_eq!(docker_contexts(&env), ["colima", "remote"]);
    }

    #[test]
    fn docker_host_tls_uses_cert_path_or_home() {
        let dir = TempDir::new().unwrap();
        let mut env = env(&dir);
        env.docker_host = Some("tcp://remote:2376".into());
        env.docker_tls_verify = Some("1".into());
        let default_dir = env.home.as_ref().unwrap().join(".docker");
        for cert_dir in [default_dir, dir.path().join("certs")] {
            if cert_dir == dir.path().join("certs") {
                env.docker_cert_path = Some(cert_dir.clone());
            }
            assert_eq!(
                discover(&env).unwrap().endpoint,
                Endpoint::Tcp {
                    url: "tcp://remote:2376".into(),
                    tls: Some(TlsConfig {
                        cert_dir,
                        verify: true
                    }),
                }
            );
        }
        env.docker_tls_verify = Some(String::new());
        assert!(matches!(
            discover(&env).unwrap().endpoint,
            Endpoint::Tcp { tls: None, .. }
        ));
        env.docker_tls_verify = Some("0".into()); // Docker treats any nonempty value as enabled.
        assert!(matches!(
            discover(&env).unwrap().endpoint,
            Endpoint::Tcp { tls: Some(_), .. }
        ));
        env.home = None;
        env.docker_cert_path = None;
        assert!(matches!(
            discover(&env),
            Err(ConnectError::InvalidConfig(_))
        ));
    }

    #[test]
    fn invalid_explicit_hosts_never_fall_through() {
        let dir = TempDir::new().unwrap();
        let mut env = env(&dir);
        for host in [
            "localhost:2375",
            "ssh://remote",
            "unix://relative",
            "tcp://",
            "http:///",
            "tcp://white space",
        ] {
            env.docker_host = Some(host.into());
            assert!(
                matches!(discover(&env), Err(ConnectError::InvalidConfig(_))),
                "{host}"
            );
        }
    }

    #[test]
    fn context_from_config_uses_sha256_directory_and_tls() {
        let dir = TempDir::new().unwrap();
        let env = env(&dir);
        write(
            &env.config_dir().unwrap().join("config.json"),
            r#"{"currentContext":"remote"}"#,
        );
        let cert_dir = context(&env, "tcp://remote:2376", true);
        fs::create_dir_all(&cert_dir).unwrap();
        assert_eq!(
            discover(&env).unwrap(),
            Discovered {
                endpoint: Endpoint::Tcp {
                    url: "tcp://remote:2376".into(),
                    tls: Some(TlsConfig {
                        cert_dir,
                        verify: false
                    })
                },
                source: EndpointSource::Context("remote".into()),
            }
        );
    }

    #[test]
    fn context_env_overrides_config_and_uses_default_config_dir() {
        let dir = TempDir::new().unwrap();
        let mut env = env(&dir);
        env.docker_config_dir = None;
        env.docker_context = Some("remote".into());
        write(
            &env.config_dir().unwrap().join("config.json"),
            "invalid json",
        );
        context(&env, "unix:///unreachable.sock", false);
        assert_eq!(
            discover(&env).unwrap(),
            Discovered {
                endpoint: Endpoint::Unix("/unreachable.sock".into()),
                source: EndpointSource::Context("remote".into()),
            }
        );
    }

    #[test]
    fn context_https_verifies_by_default() {
        let dir = TempDir::new().unwrap();
        let mut env = env(&dir);
        env.docker_context = Some("remote".into());
        let cert_dir = context(&env, "https://remote:2376", false);
        assert_eq!(
            discover(&env).unwrap().endpoint,
            Endpoint::Tcp {
                url: "https://remote:2376".into(),
                tls: Some(TlsConfig {
                    cert_dir,
                    verify: true
                }),
            }
        );
    }

    #[test]
    fn broken_context_and_config_are_specific_errors() {
        let dir = TempDir::new().unwrap();
        let mut env = env(&dir);
        env.docker_context = Some("missing".into());
        assert!(matches!(
            discover(&env),
            Err(ConnectError::InvalidConfig(_))
        ));
        env.docker_context = None;
        write(&env.config_dir().unwrap().join("config.json"), "bad json");
        assert!(matches!(
            discover(&env),
            Err(ConnectError::InvalidConfig(_))
        ));
        env.docker_context = Some("remote".into());
        context(&env, "ssh://remote", false);
        assert!(matches!(
            discover(&env),
            Err(ConnectError::InvalidConfig(_))
        ));
    }

    #[test]
    fn tcp_probe_is_explicitly_unsupported() {
        assert!(matches!(
            probe(&Endpoint::Tcp {
                url: "tcp://remote:2375".into(),
                tls: None
            }),
            Err(ConnectError::InvalidConfig(_))
        ));
    }

    #[cfg(unix)]
    mod unix {
        use super::*;
        use std::os::unix::{
            fs::{MetadataExt, PermissionsExt},
            net::UnixListener,
        };

        fn listen(path: &std::path::Path) -> UnixListener {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            UnixListener::bind(path).unwrap()
        }

        #[test]
        fn probe_classifies_missing_refused_and_connectable() {
            let dir = TempDir::new().unwrap();
            let endpoint = Endpoint::Unix(dir.path().join("socket"));
            assert_eq!(
                probe(&endpoint),
                Err(ConnectError::NotFound {
                    tried: vec![endpoint.to_string()]
                })
            );
            let listener = listen(&dir.path().join("socket"));
            assert_eq!(probe(&endpoint), Ok(()));
            drop(listener);
            assert_eq!(
                probe(&endpoint),
                Err(ConnectError::Refused {
                    endpoint: endpoint.to_string()
                })
            );
        }

        #[test]
        fn fallbacks_are_probed_in_order() {
            let dir = TempDir::new().unwrap();
            let env = env(&dir);
            let runtime = env.xdg_runtime_dir.as_ref().unwrap();
            let rootful = listen(&env.rootful_socket);
            let rootless = listen(&runtime.join("docker.sock"));
            let _podman = listen(&runtime.join("podman/podman.sock"));
            assert_eq!(
                discover(&env).unwrap().source,
                EndpointSource::Default("rootful Docker")
            );
            drop(rootful); // Refused sockets must not prevent a later working fallback.
            assert_eq!(
                discover(&env).unwrap().source,
                EndpointSource::Default("rootless Docker")
            );
            drop(rootless);
            fs::remove_file(runtime.join("docker.sock")).unwrap();
            assert_eq!(
                discover(&env).unwrap(),
                Discovered {
                    endpoint: Endpoint::Unix(runtime.join("podman/podman.sock")),
                    source: EndpointSource::Default("Podman"),
                }
            );
        }

        #[test]
        fn default_context_skips_metadata_and_uses_fallbacks() {
            let dir = TempDir::new().unwrap();
            let mut env = env(&dir);
            let _listener = listen(&env.rootful_socket);
            write(
                &env.config_dir().unwrap().join("config.json"),
                r#"{"currentContext":"default"}"#,
            );
            assert_eq!(
                discover(&env).unwrap().endpoint,
                Endpoint::Unix(env.rootful_socket.clone())
            );
            env.docker_context = Some("default".into());
            write(
                &env.config_dir().unwrap().join("config.json"),
                "invalid json",
            );
            assert_eq!(
                discover(&env).unwrap().source,
                EndpointSource::Default("rootful Docker")
            );
        }

        #[test]
        fn not_found_lists_every_candidate() {
            let dir = TempDir::new().unwrap();
            let mut env = env(&dir);
            env.home = None; // No platform home fallbacks.
            let runtime = env.xdg_runtime_dir.as_ref().unwrap();
            assert_eq!(
                discover(&env),
                Err(ConnectError::NotFound {
                    tried: vec![
                        Endpoint::Unix(env.rootful_socket.clone()).to_string(),
                        Endpoint::Unix(runtime.join("docker.sock")).to_string(),
                        Endpoint::Unix(runtime.join("podman/podman.sock")).to_string(),
                    ]
                })
            );
            env.xdg_runtime_dir = None;
            assert_eq!(
                discover(&env),
                Err(ConnectError::NotFound {
                    tried: vec![Endpoint::Unix(env.rootful_socket).to_string()]
                })
            );
        }

        #[test]
        fn permission_denied_is_preferred_unless_later_socket_works() {
            let dir = TempDir::new().unwrap();
            // The temporary directory is owned by the effective UID; root
            // bypasses socket permissions and cannot exercise EACCES.
            if fs::metadata(dir.path()).unwrap().uid() == 0 {
                return;
            }
            let env = env(&dir);
            let _rootful = listen(&env.rootful_socket);
            fs::set_permissions(&env.rootful_socket, fs::Permissions::from_mode(0o000)).unwrap();
            let expected = ConnectError::PermissionDenied {
                endpoint: Endpoint::Unix(env.rootful_socket.clone()).to_string(),
            };
            assert_eq!(
                probe(&Endpoint::Unix(env.rootful_socket.clone())),
                Err(expected.clone())
            );
            assert_eq!(discover(&env), Err(expected));
            let _rootless = listen(&env.xdg_runtime_dir.as_ref().unwrap().join("docker.sock"));
            assert_eq!(
                discover(&env).unwrap().source,
                EndpointSource::Default("rootless Docker")
            );
        }
    }
}
