//! SSH remotes, as `docker -H ssh://…` does them: the system OpenSSH client
//! runs `docker system dial-stdio` on the remote and carries the API over
//! its stdin/stdout. Using the system client means every ssh config option
//! (aliases, `ProxyJump`, agents, an existing `ControlMaster`) works as in a
//! terminal.
//!
//! `ssh` never prompts: Berth has no terminal to type into, so host keys,
//! passwords and key passphrases must be settled before connecting.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Shown wherever an SSH endpoint can't be used for want of a client.
pub const SSH_CLIENT_MISSING: &str =
    "SSH remotes need the OpenSSH client (ssh), which wasn't found on this machine.";

/// The system `ssh` executable, looked up once: `PATH` first, then the usual
/// install locations, as apps started from a desktop may get a minimal `PATH`.
#[must_use]
pub fn ssh_client() -> Option<&'static Path> {
    static CLIENT: OnceLock<Option<PathBuf>> = OnceLock::new();
    CLIENT.get_or_init(find_ssh_client).as_deref()
}

fn find_ssh_client() -> Option<PathBuf> {
    let name = if cfg!(windows) { "ssh.exe" } else { "ssh" };
    let path = std::env::var_os("PATH").unwrap_or_default();
    let fallbacks: Vec<PathBuf> = if cfg!(windows) {
        std::env::var_os("SystemRoot")
            .map(|root| PathBuf::from(root).join(r"System32\OpenSSH\ssh.exe"))
            .into_iter()
            .collect()
    } else {
        [
            "/usr/bin/ssh",
            "/usr/local/bin/ssh",
            "/opt/homebrew/bin/ssh",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .chain(fallbacks)
        .find(|candidate| candidate.is_file())
}

/// Arguments for the `ssh` process that carries one API connection.
#[must_use]
pub fn dial_args(destination: &str, port: Option<u16>) -> Vec<String> {
    let mut args: Vec<String> = [
        // A raw byte stream: never a PTY, even if the config asks for one.
        "-T",
        // No prompts: fail instead of waiting on a terminal nobody sees.
        "-o",
        "BatchMode=yes",
        // A dead network or host fails in seconds rather than hanging.
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=3",
        // Use an existing ControlMaster but never become one: closing an
        // idle connection would otherwise drop every session sharing it.
        "-o",
        "ControlMaster=no",
    ]
    .map(String::from)
    .into();
    args.extend(target_args(destination, port));
    args.extend(["docker", "system", "dial-stdio"].map(String::from));
    args
}

/// Arguments that print the resolved ssh config for the destination.
#[must_use]
pub fn config_args(destination: &str, port: Option<u16>) -> Vec<String> {
    let mut args = vec!["-G".to_owned()];
    args.extend(target_args(destination, port));
    args
}

fn target_args(destination: &str, port: Option<u16>) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(port) = port {
        args.extend(["-p".to_owned(), port.to_string()]);
    }
    // `--` so the destination is never read as an option.
    args.extend(["--".to_owned(), destination.to_owned()]);
    args
}

/// The `hostname` line of `ssh -G` output.
#[must_use]
pub fn resolved_hostname(config: &str) -> Option<String> {
    config.lines().find_map(|line| {
        let (key, value) = line.split_once(' ')?;
        (key == "hostname" && !value.trim().is_empty()).then(|| value.trim().to_owned())
    })
}

/// What to tell the user when `ssh` ended before the engine answered.
#[must_use]
pub fn failure_message(destination: &str, port: Option<u16>, stderr: &str) -> String {
    let login = port.map_or_else(
        || format!("ssh {destination}"),
        |port| format!("ssh -p {port} {destination}"),
    );
    let lower = stderr.to_ascii_lowercase();
    if lower.contains("remote host identification has changed") {
        format!(
            "{destination}'s host key has changed. If that's expected, remove the old key \
             from known_hosts and run `{login}` in a terminal to trust the new one."
        )
    } else if lower.contains("host key verification failed") {
        format!(
            "{destination}'s host key isn't trusted yet. Run `{login}` in a terminal once to \
             check and save it."
        )
    } else if lower.contains("permission denied")
        && (lower.contains("docker daemon socket") || lower.contains("dial unix"))
    {
        format!(
            "Your user on {destination} isn't allowed to use the Docker socket. Add it to the \
             docker group there, or use rootless Docker."
        )
    } else if lower.contains("permission denied") {
        format!(
            "{destination} refused the SSH login. Berth can't type passwords or key \
             passphrases: load your key into ssh-agent, then check that `{login}` logs in \
             without asking for anything."
        )
    } else if lower.contains("docker")
        && (lower.contains("command not found")
            || lower.contains("docker: not found")
            || lower.contains("is not recognized"))
    {
        format!(
            "The docker CLI isn't installed on {destination}. Berth needs it there to reach \
             the engine (docker system dial-stdio)."
        )
    } else {
        // The server's reason (e.g. a Tailscale policy) comes before ssh's
        // own "Connection closed by …"; warnings aren't the reason.
        let lines = || {
            stderr
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
        };
        lines()
            .find(|line| !line.starts_with("Warning:") && !line.starts_with("Connection closed by"))
            .or_else(|| lines().next_back())
            .map_or_else(
                || format!("{login} closed the connection before the engine answered."),
                |line| format!("{login}: {line}"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_is_never_an_option() {
        assert_eq!(
            dial_args("me@box", Some(2222))
                .iter()
                .skip_while(|arg| *arg != "-p")
                .collect::<Vec<_>>(),
            [
                "-p",
                "2222",
                "--",
                "me@box",
                "docker",
                "system",
                "dial-stdio"
            ]
        );
        assert!(dial_args("box", None).contains(&"BatchMode=yes".to_owned()));
        assert!(dial_args("box", None).contains(&"ControlMaster=no".to_owned()));
        assert_eq!(
            dial_args("box", None).first().map(String::as_str),
            Some("-T")
        );
        assert_eq!(config_args("box", None), ["-G", "--", "box"]);
    }

    #[test]
    fn hostname_comes_from_ssh_config_output() {
        let config = "user me\nhostname prod.example.com\nport 22\n";
        assert_eq!(
            resolved_hostname(config).as_deref(),
            Some("prod.example.com")
        );
        assert_eq!(resolved_hostname("user me\n"), None);
    }

    #[test]
    fn failures_say_what_to_do() {
        let message = |stderr| failure_message("prod", None, stderr);
        assert!(message("Host key verification failed.\r\n").contains("`ssh prod`"));
        assert!(
            message("@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@")
                .contains("has changed")
        );
        assert!(message("me@prod: Permission denied (publickey).").contains("ssh-agent"));
        assert!(message("bash: line 1: docker: command not found").contains("docker CLI"));
        assert!(message("sh: 1: docker: not found").contains("docker CLI"));
        assert!(
            message(
                "permission denied while trying to connect to the Docker daemon socket at \
                 unix:///var/run/docker.sock"
            )
            .contains("docker group")
        );
        assert!(
            message(
                "failed to open the raw stream connection: dial unix /var/run/docker.sock: \
                 connect: permission denied"
            )
            .contains("docker group")
        );
        assert_eq!(
            message("ssh: Could not resolve hostname prod: Name or service not known\n"),
            "ssh prod: ssh: Could not resolve hostname prod: Name or service not known"
        );
        assert_eq!(
            message(
                "tailscale: tailnet policy does not permit you to SSH as user \"me\"\n\
                 Connection closed by 100.64.0.1 port 22\n"
            ),
            "ssh prod: tailscale: tailnet policy does not permit you to SSH as user \"me\""
        );
        assert_eq!(
            message("Connection closed by 100.64.0.1 port 22"),
            "ssh prod: Connection closed by 100.64.0.1 port 22"
        );
        assert!(failure_message("prod", Some(2222), "").starts_with("ssh -p 2222 prod closed"));
    }
}
