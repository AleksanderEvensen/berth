//! Grouping containers into the list shown on the Containers screen.

use std::collections::BTreeMap;

use crate::Container;

/// A compose project and its containers, ordered by service name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stack<'a> {
    pub project: &'a str,
    pub containers: Vec<&'a Container>,
}

impl Stack<'_> {
    pub fn active_count(&self) -> usize {
        self.containers.iter().filter(|c| c.state.is_active()).count()
    }

    /// A stack belongs to the "Running" section if any service is active.
    pub fn is_active(&self) -> bool {
        self.active_count() > 0
    }

    pub fn working_dir(&self) -> Option<&str> {
        self.containers
            .iter()
            .find_map(|c| c.compose.as_ref()?.working_dir.as_deref())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node<'a> {
    Stack(Stack<'a>),
    Container(&'a Container),
}

impl Node<'_> {
    pub fn is_active(&self) -> bool {
        match self {
            Node::Stack(s) => s.is_active(),
            Node::Container(c) => c.state.is_active(),
        }
    }

    fn sort_key(&self) -> &str {
        match self {
            Node::Stack(s) => s.project,
            Node::Container(c) => &c.name,
        }
    }
}

/// The Containers list: active nodes first, then stopped, each alphabetical.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContainerTree<'a> {
    pub running: Vec<Node<'a>>,
    pub stopped: Vec<Node<'a>>,
}

impl<'a> ContainerTree<'a> {
    pub fn build(containers: &'a [Container]) -> Self {
        let mut stacks: BTreeMap<&str, Vec<&Container>> = BTreeMap::new();
        let mut nodes = Vec::new();
        for c in containers {
            match &c.compose {
                Some(compose) => stacks.entry(&compose.project).or_default().push(c),
                None => nodes.push(Node::Container(c)),
            }
        }
        for (project, mut members) in stacks {
            members.sort_by(|a, b| service(a).cmp(service(b)));
            nodes.push(Node::Stack(Stack {
                project,
                containers: members,
            }));
        }
        nodes.sort_by(|a, b| a.sort_key().cmp(b.sort_key()));

        let (running, stopped) = nodes.into_iter().partition(Node::is_active);
        Self { running, stopped }
    }

    pub fn running_container_count(&self) -> usize {
        self.running
            .iter()
            .map(|n| match n {
                Node::Stack(s) => s.active_count(),
                Node::Container(_) => 1,
            })
            .sum()
    }
}

fn service(c: &Container) -> &str {
    c.compose.as_ref().map_or(&c.name, |cr| &cr.service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContainerState, fixtures};

    #[test]
    fn groups_compose_projects_and_splits_by_activity() {
        let containers = fixtures::containers();
        let tree = ContainerTree::build(&containers);

        let running: Vec<_> = tree.running.iter().map(Node::sort_key).collect();
        let stopped: Vec<_> = tree.stopped.iter().map(Node::sort_key).collect();
        assert_eq!(running, ["berth-dev", "rustfs", "tihlde-voting"]);
        assert_eq!(stopped, ["mailpit-scratch", "photon_dev"]);
    }

    #[test]
    fn a_stack_with_one_running_service_is_running() {
        let mut containers = fixtures::containers();
        for c in &mut containers {
            if c.name == "photon_dev_redis" {
                c.state = ContainerState::Running;
            }
        }
        let tree = ContainerTree::build(&containers);
        let Some(Node::Stack(stack)) = tree
            .running
            .iter()
            .find(|n| n.sort_key() == "photon_dev")
        else {
            panic!("photon_dev should be running");
        };
        assert_eq!(stack.active_count(), 1);
        assert_eq!(stack.containers.len(), 4);
    }

    #[test]
    fn services_are_ordered_within_a_stack() {
        let containers = fixtures::containers();
        let tree = ContainerTree::build(&containers);
        let Some(Node::Stack(stack)) = tree.stopped.iter().find(|n| n.sort_key() == "photon_dev")
        else {
            panic!()
        };
        let services: Vec<_> = stack.containers.iter().map(|c| service(c)).collect();
        assert_eq!(services, ["db", "mail_sink", "redis", "rustfs"]);
    }
}
