//! Fix-it guide for an unreachable engine socket (A5).
//!
//! Berth runs nothing itself: it explains the options with copyable
//! commands, and the store's reconnect loop picks up a socket once one works.

use berth_docker::{ConnectError, EngineError};
use berth_ui::{Heading, IconButton, Theme, icons};
use gpui::{ClipboardItem, FontWeight, Hsla, IntoElement, SharedString, div, prelude::*, px};

use crate::store::{Connection, Pin};

/// Why the engine can't be used, when there is something to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    cause: Cause,
    pin: Option<Pin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cause {
    PermissionDenied { endpoint: String },
    NotFound { tried: Vec<String> },
}

impl Problem {
    pub fn of(connection: &Connection) -> Option<Self> {
        let Connection::Disconnected { error, pin } = connection else {
            return None;
        };
        let cause = match error {
            EngineError::Connect(ConnectError::PermissionDenied { endpoint }) => {
                Cause::PermissionDenied {
                    endpoint: endpoint.clone(),
                }
            }
            EngineError::Connect(ConnectError::NotFound { tried }) => Cause::NotFound {
                tried: tried.clone(),
            },
            _ => return None,
        };
        Some(Self {
            cause,
            pin: pin.clone(),
        })
    }

    /// One line for tight spots like the sidebar footer.
    pub const fn short(&self) -> &'static str {
        match self.cause {
            Cause::PermissionDenied { .. } => "Permission denied · How to fix",
            Cause::NotFound { .. } => "No engine found · How to fix",
        }
    }

    const fn title(&self) -> &'static str {
        match self.cause {
            Cause::PermissionDenied { .. } => "Berth can't open the engine socket",
            Cause::NotFound { .. } => "No container engine found",
        }
    }

    fn intro(&self) -> String {
        let cause = match &self.cause {
            Cause::PermissionDenied { endpoint } => {
                format!("Your user isn't allowed to use {endpoint}.")
            }
            Cause::NotFound { tried } => {
                format!("Berth looked at {} and found no engine.", tried.join(", "))
            }
        };
        format!("{cause} {}", self.recovery())
    }

    /// How Berth gets from a fixed engine to a connection.
    fn recovery(&self) -> String {
        const AFTER: &str = "so Berth keeps retrying it and won't find a socket elsewhere. \
                             If your fix puts the engine on another socket,";
        match &self.pin {
            None => "Set up one of the options below; Berth keeps retrying and connects as \
                     soon as one of them works."
                .into(),
            Some(Pin::Settings) => format!(
                "Settings → Engine pins Berth to this endpoint, {AFTER} set Connect to \
                 back to Auto."
            ),
            Some(Pin::DockerHost) => format!(
                "DOCKER_HOST pins Berth to this endpoint, {AFTER} unset DOCKER_HOST and \
                 restart Berth, or enter the new socket under Custom in Settings → Engine."
            ),
            Some(Pin::ContextVar(name)) => format!(
                "DOCKER_CONTEXT selects the “{name}” context, which pins Berth to this \
                 endpoint, {AFTER} unset DOCKER_CONTEXT and restart Berth, or enter the new \
                 socket under Custom in Settings → Engine."
            ),
            Some(Pin::CurrentContext(name)) => format!(
                "The docker CLI's current context, “{name}”, pins Berth to this endpoint, \
                 {AFTER} run `docker context use default`, or enter the new socket under \
                 Custom in Settings → Engine."
            ),
        }
    }

    const fn fixes(&self) -> [Fix; 3] {
        let last = match self.cause {
            Cause::PermissionDenied { .. } => Fix {
                title: "Join the docker group",
                tag: Some(Tag::Warning("Root-equivalent")),
                description: "Lets your user use the system-wide Docker daemon. Anything \
                              running as you can then get root on this machine without a \
                              password, so only do this on a machine you trust.",
                steps: &[Step {
                    label: None,
                    commands: &["sudo usermod -aG docker $USER"],
                }],
                note: Some("Log out and back in for the new group to apply."),
            },
            Cause::NotFound { .. } => Fix {
                title: "Start the system Docker daemon",
                tag: Some(Tag::Warning("Root-equivalent")),
                description: "Runs Docker as root for the whole machine. Using it without \
                              sudo needs the docker group, which gives anything running as \
                              you root on this machine without a password.",
                steps: &[Step {
                    label: None,
                    commands: &[
                        "sudo systemctl enable --now docker",
                        "sudo usermod -aG docker $USER",
                    ],
                }],
                note: Some("Log out and back in for the new group to apply."),
            },
        };
        [
            Fix {
                title: "Rootless Docker",
                tag: Some(Tag::Accent("Recommended")),
                description: "Runs the Docker daemon as your user instead of root. Containers \
                              can still reach what your user can, such as files you \
                              bind-mount, but a mistake or a bad image can't gain root on \
                              this machine through Docker.",
                steps: &[
                    Step {
                        label: Some(
                            "Debian, Ubuntu, Fedora: install docker-ce-rootless-extras (and \
                             uidmap on Debian/Ubuntu) from Docker's repository, then",
                        ),
                        commands: &["dockerd-rootless-setuptool.sh install"],
                    },
                    Step {
                        label: Some(
                            "Arch: install docker-rootless-extras from the AUR. It needs at \
                             least 65536 subordinate IDs for you in both /etc/subuid and \
                             /etc/subgid; check with",
                        ),
                        commands: &["grep \"^$USER:\" /etc/subuid /etc/subgid"],
                    },
                    Step {
                        label: Some(
                            "If either file has no line for you, add one with a range no \
                             other line overlaps (the ArchWiki's Docker page, \"Rootless \
                             Docker daemon\", shows how). Then",
                        ),
                        commands: &["systemctl --user enable --now docker.socket"],
                    },
                ],
                note: None,
            },
            Fix {
                title: "Rootless Podman",
                tag: None,
                description: "Podman's Docker-compatible API, also running as your user \
                              instead of root. Docker Compose works against it too.",
                steps: &[Step {
                    label: None,
                    commands: &["systemctl --user enable --now podman.socket"],
                }],
                note: Some("Needs Podman installed."),
            },
            last,
        ]
    }
}

#[derive(Clone, Copy)]
enum Tag {
    Accent(&'static str),
    Warning(&'static str),
}

struct Fix {
    title: &'static str,
    tag: Option<Tag>,
    description: &'static str,
    steps: &'static [Step],
    note: Option<&'static str>,
}

/// Commands to run in order, optionally introduced by a line saying when.
struct Step {
    label: Option<&'static str>,
    commands: &'static [&'static str],
}

/// The full guide: title, what went wrong, and the options in order.
pub fn engine_help(problem: &Problem, theme: &Theme) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .pt(px(Theme::SPACE_LG))
                .text_size(px(Theme::TEXT_LG))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(problem.title()),
        )
        .child(
            div()
                .pt(px(Theme::SPACE_XS))
                .text_size(px(Theme::TEXT_MD))
                .text_color(theme.text_muted)
                .child(problem.intro()),
        )
        .children(
            problem
                .fixes()
                .into_iter()
                .zip(1_usize..)
                .map(|(fix, number)| render_fix(number, &fix, theme)),
        )
}

fn render_fix(number: usize, fix: &Fix, theme: &Theme) -> impl IntoElement + use<> {
    let tag = fix.tag.map(|tag| {
        let (label, color) = match tag {
            Tag::Accent(label) => (label, theme.accent),
            Tag::Warning(label) => (label, theme.warning),
        };
        render_tag(label, color)
    });
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(Theme::SPACE_SM))
                .child(Heading::new(format!("{number}. {}", fix.title)))
                .children(tag.map(|tag| div().pt(px(Theme::SPACE_LG - 4.0)).child(tag))),
        )
        .child(
            div()
                .px(px(Theme::SPACE_XS))
                .pb(px(Theme::SPACE_SM))
                .text_size(px(Theme::TEXT_MD))
                .text_color(theme.text_muted)
                .child(fix.description),
        )
        .children(fix.steps.iter().map(|step| render_step(step, theme)))
        .children(fix.note.map(|note| {
            div()
                .px(px(Theme::SPACE_XS))
                .pt(px(Theme::SPACE_SM))
                .text_size(px(Theme::TEXT_SM))
                .text_color(theme.text_faint)
                .child(note)
        }))
}

fn render_step(step: &Step, theme: &Theme) -> impl IntoElement + use<> {
    div()
        .flex()
        .flex_col()
        .children(step.label.map(|label| {
            div()
                .px(px(Theme::SPACE_XS))
                .pt(px(Theme::SPACE_XS))
                .pb(px(Theme::SPACE_SM))
                .text_size(px(Theme::TEXT_SM))
                .text_color(theme.text_muted)
                .child(label)
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .rounded(px(Theme::PANEL_RADIUS))
                .bg(theme.surface_card)
                .border_1()
                .border_color(theme.border)
                .overflow_hidden()
                .children(
                    step.commands
                        .iter()
                        .enumerate()
                        .map(|(ix, command)| render_command(ix, command, theme)),
                ),
        )
}

fn render_command(ix: usize, command: &'static str, theme: &Theme) -> impl IntoElement + use<> {
    div()
        .h(px(38.0))
        .pl(px(Theme::SPACE_MD))
        .pr(px(Theme::SPACE_SM))
        .flex()
        .items_center()
        .gap(px(Theme::SPACE_SM))
        .when(ix > 0, |d| d.border_t_1().border_color(theme.border))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(Theme::TEXT_MD))
                .font_family(theme.font_mono.clone())
                .text_color(theme.text)
                .child(command),
        )
        .child(
            IconButton::new(
                SharedString::from(format!("copy-{command}")),
                icons::COPY,
                "Copy command",
            )
            .size(24.0, 14.0)
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(command.to_string()));
            }),
        )
}

fn render_tag(label: &'static str, color: Hsla) -> impl IntoElement {
    div()
        .px(px(6.0))
        .rounded(px(Theme::CONTROL_RADIUS))
        .border_1()
        .border_color(color.opacity(0.5))
        .text_size(px(Theme::TEXT_XS))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOCKET: &str = "unix:///var/run/docker.sock";

    fn denied(pin: Option<Pin>) -> Connection {
        Connection::Disconnected {
            error: EngineError::Connect(ConnectError::PermissionDenied {
                endpoint: SOCKET.into(),
            }),
            pin,
        }
    }

    fn commands(fix: &Fix) -> impl Iterator<Item = &'static str> {
        fix.steps.iter().flat_map(|s| s.commands.iter().copied())
    }

    #[test]
    fn only_socket_problems_get_a_guide() {
        let problem = Problem::of(&denied(None)).expect("a guide");
        assert_eq!(
            problem.cause,
            Cause::PermissionDenied {
                endpoint: SOCKET.into()
            }
        );
        let missing = Connection::Disconnected {
            error: EngineError::Connect(ConnectError::NotFound {
                tried: vec![SOCKET.into()],
            }),
            pin: None,
        };
        assert!(matches!(
            Problem::of(&missing).map(|p| p.cause),
            Some(Cause::NotFound { .. })
        ));
        let refused = Connection::Disconnected {
            error: EngineError::Connect(ConnectError::Refused {
                endpoint: "tcp://localhost:2375".into(),
            }),
            pin: None,
        };
        assert_eq!(Problem::of(&refused), None);
        assert_eq!(Problem::of(&Connection::Connecting), None);
    }

    fn missing(pin: Option<Pin>) -> Connection {
        Connection::Disconnected {
            error: EngineError::Connect(ConnectError::NotFound {
                tried: vec!["unix:///missing.sock".into()],
            }),
            pin,
        }
    }

    #[test]
    fn only_discovery_promises_to_reconnect_by_itself() {
        let recovery =
            |connection: Connection| Problem::of(&connection).expect("a guide").recovery();
        assert!(recovery(denied(None)).contains("connects as soon as"));
        assert!(recovery(missing(None)).contains("connects as soon as"));
        // A pinned socket that is missing stays missing whichever fix runs,
        // the system daemon included, so the switch applies to every option.
        for (pin, fix) in [
            (Pin::Settings, "back to Auto"),
            (Pin::DockerHost, "unset DOCKER_HOST"),
            (Pin::ContextVar("ci".into()), "unset DOCKER_CONTEXT"),
            (
                Pin::CurrentContext("ci".into()),
                "docker context use default",
            ),
        ] {
            for connection in [denied(Some(pin.clone())), missing(Some(pin.clone()))] {
                let text = recovery(connection);
                assert!(!text.contains("connects as soon as"), "{text}");
                assert!(!text.contains("option"), "{text}");
                assert!(text.contains(fix), "{text}");
            }
        }
    }

    #[test]
    fn the_docker_group_comes_last_and_is_flagged() {
        for connection in [denied(None), missing(None)] {
            let fixes = Problem::of(&connection).expect("a guide").fixes();
            assert!(matches!(fixes[0].tag, Some(Tag::Accent(_))));
            let last = &fixes[2];
            assert!(matches!(last.tag, Some(Tag::Warning("Root-equivalent"))));
            assert!(commands(last).any(|c| c.contains("usermod -aG docker")));
            assert!(
                fixes[..2]
                    .iter()
                    .all(|f| commands(f).all(|c| !c.contains("-aG docker")))
            );
            // Subordinate ID ranges must not be guessed: they can collide.
            assert!(
                fixes
                    .iter()
                    .flat_map(commands)
                    .all(|c| !c.contains("--add-sub"))
            );
        }
    }
}
