//! Fix-it guide for an unreachable engine socket (A5).
//!
//! Berth runs nothing itself: it explains the options with copyable
//! commands, and the store's reconnect loop picks up a socket once one works.

use berth_docker::{ConnectError, EngineError};
use berth_ui::{Heading, IconButton, Theme, icons};
use gpui::{ClipboardItem, FontWeight, Hsla, IntoElement, SharedString, div, prelude::*, px};

/// Why the engine socket can't be used, when there is something to fix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    PermissionDenied { endpoint: String },
    NotFound { tried: Vec<String> },
}

impl Problem {
    pub fn of(error: &EngineError) -> Option<Self> {
        match error {
            EngineError::Connect(ConnectError::PermissionDenied { endpoint }) => {
                Some(Self::PermissionDenied {
                    endpoint: endpoint.clone(),
                })
            }
            EngineError::Connect(ConnectError::NotFound { tried }) => Some(Self::NotFound {
                tried: tried.clone(),
            }),
            _ => None,
        }
    }

    /// One line for tight spots like the sidebar footer.
    pub const fn short(&self) -> &'static str {
        match self {
            Self::PermissionDenied { .. } => "Permission denied · How to fix",
            Self::NotFound { .. } => "No engine found · How to fix",
        }
    }

    const fn title(&self) -> &'static str {
        match self {
            Self::PermissionDenied { .. } => "Berth can't open the engine socket",
            Self::NotFound { .. } => "No container engine found",
        }
    }

    fn intro(&self) -> String {
        match self {
            Self::PermissionDenied { endpoint } => format!(
                "Your user isn't allowed to use {endpoint}. Pick one of the options below; \
                 Berth keeps retrying and connects as soon as a socket works."
            ),
            Self::NotFound { tried } => format!(
                "Berth looked at {}. Install and start one of the engines below; \
                 Berth keeps retrying and connects as soon as a socket works.",
                tried.join(", ")
            ),
        }
    }

    const fn fixes(&self) -> [Fix; 3] {
        let last = match self {
            Self::PermissionDenied { .. } => Fix {
                title: "Join the docker group",
                tag: Some(Tag::Warning("Root-equivalent")),
                description: "Lets your user use the system-wide Docker daemon. Anything \
                              running as you can then get root on this machine without a \
                              password, so only do this on a machine you trust.",
                commands: &["sudo usermod -aG docker $USER"],
                note: Some("Log out and back in for the new group to apply."),
            },
            Self::NotFound { .. } => Fix {
                title: "Start the system Docker daemon",
                tag: Some(Tag::Warning("Root-equivalent")),
                description: "Runs Docker as root for the whole machine. Using it without \
                              sudo needs the docker group, which gives anything running as \
                              you root on this machine without a password.",
                commands: &[
                    "sudo systemctl enable --now docker",
                    "sudo usermod -aG docker $USER",
                ],
                note: Some("Log out and back in for the new group to apply."),
            },
        };
        [
            Fix {
                title: "Rootless Docker",
                tag: Some(Tag::Accent("Recommended")),
                description: "Runs the Docker daemon as your user, so containers can't touch \
                              the rest of the system.",
                commands: &["dockerd-rootless-setuptool.sh install"],
                note: Some(
                    "Needs your distribution's rootless extras package \
                     (docker-ce-rootless-extras, or docker-rootless-extras on Arch).",
                ),
            },
            Fix {
                title: "Rootless Podman",
                tag: None,
                description: "Podman's Docker-compatible API, also running as your user. \
                              Docker Compose works against it too.",
                commands: &["systemctl --user enable --now podman.socket"],
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
    commands: &'static [&'static str],
    note: Option<&'static str>,
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
                    fix.commands
                        .iter()
                        .enumerate()
                        .map(|(ix, command)| render_command(ix, command, theme)),
                ),
        )
        .children(fix.note.map(|note| {
            div()
                .px(px(Theme::SPACE_XS))
                .pt(px(Theme::SPACE_SM))
                .text_size(px(Theme::TEXT_SM))
                .text_color(theme.text_faint)
                .child(note)
        }))
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
            IconButton::new(SharedString::from(format!("copy-{command}")), icons::COPY)
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

    #[test]
    fn only_socket_problems_get_a_guide() {
        let denied = EngineError::Connect(ConnectError::PermissionDenied {
            endpoint: "unix:///var/run/docker.sock".into(),
        });
        assert_eq!(
            Problem::of(&denied),
            Some(Problem::PermissionDenied {
                endpoint: "unix:///var/run/docker.sock".into()
            })
        );
        let missing = EngineError::Connect(ConnectError::NotFound {
            tried: vec!["unix:///var/run/docker.sock".into()],
        });
        assert!(matches!(
            Problem::of(&missing),
            Some(Problem::NotFound { .. })
        ));
        let refused = EngineError::Connect(ConnectError::Refused {
            endpoint: "tcp://localhost:2375".into(),
        });
        assert_eq!(Problem::of(&refused), None);
    }

    #[test]
    fn the_docker_group_comes_last_and_is_flagged() {
        for problem in [
            Problem::PermissionDenied {
                endpoint: "unix:///var/run/docker.sock".into(),
            },
            Problem::NotFound { tried: Vec::new() },
        ] {
            let fixes = problem.fixes();
            assert!(matches!(fixes[0].tag, Some(Tag::Accent(_))));
            let last = &fixes[2];
            assert!(matches!(last.tag, Some(Tag::Warning("Root-equivalent"))));
            assert!(
                last.commands
                    .iter()
                    .any(|c| c.contains("usermod -aG docker"))
            );
            assert!(
                fixes[..2]
                    .iter()
                    .all(|f| f.commands.iter().all(|c| !c.contains("sudo")))
            );
        }
    }
}
