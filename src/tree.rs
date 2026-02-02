use crate::session::Session;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum TreeItem {
    Project {
        name: String,
        expanded: bool,
    },
    Session {
        session: Session,
        project: Option<String>,
    },
}

impl TreeItem {
    pub fn is_project(&self) -> bool {
        matches!(self, TreeItem::Project { .. })
    }

    #[allow(dead_code)]
    pub fn is_session(&self) -> bool {
        matches!(self, TreeItem::Session { .. })
    }

    #[allow(dead_code)]
    pub fn project_name(&self) -> Option<&str> {
        match self {
            TreeItem::Project { name, .. } => Some(name),
            TreeItem::Session { project, .. } => project.as_deref(),
        }
    }

    pub fn session(&self) -> Option<&Session> {
        match self {
            TreeItem::Session { session, .. } => Some(session),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn toggle_expanded(&mut self) {
        if let TreeItem::Project { expanded, .. } = self {
            *expanded = !*expanded;
        }
    }

    #[allow(dead_code)]
    pub fn is_expanded(&self) -> bool {
        match self {
            TreeItem::Project { expanded, .. } => *expanded,
            _ => false,
        }
    }
}

pub struct SessionTree {
    pub items: Vec<TreeItem>,
    collapsed_projects: HashMap<String, bool>,
}

impl SessionTree {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            collapsed_projects: HashMap::new(),
        }
    }

    pub fn build_from_sessions(&mut self, sessions: Vec<Session>) {
        // Group sessions by project
        let mut projects: HashMap<Option<String>, Vec<Session>> = HashMap::new();

        for session in sessions {
            let project = session.project.clone();
            projects.entry(project).or_default().push(session);
        }

        // Build tree structure
        self.items.clear();

        // Sort project names (None/"no project" goes last)
        let mut project_names: Vec<Option<String>> = projects.keys().cloned().collect();
        project_names.sort_by(|a, b| match (a, b) {
            (None, None) => std::cmp::Ordering::Equal,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (Some(_), None) => std::cmp::Ordering::Less,
            (Some(a), Some(b)) => a.cmp(b),
        });

        // Build tree items
        for project_name in project_names {
            let sessions = projects.get(&project_name).unwrap();

            if let Some(name) = &project_name {
                // Check if this project was previously collapsed
                let expanded = !self.collapsed_projects.get(name).copied().unwrap_or(false);

                // Add project item
                self.items.push(TreeItem::Project {
                    name: name.clone(),
                    expanded,
                });

                // Add sessions if expanded
                if expanded {
                    for session in sessions {
                        self.items.push(TreeItem::Session {
                            session: session.clone(),
                            project: Some(name.clone()),
                        });
                    }
                }
            } else {
                // No project - add sessions directly under "(no project)" header
                self.items.push(TreeItem::Project {
                    name: "(no project)".to_string(),
                    expanded: !self
                        .collapsed_projects
                        .get("(no project)")
                        .copied()
                        .unwrap_or(false),
                });

                if !self
                    .collapsed_projects
                    .get("(no project)")
                    .copied()
                    .unwrap_or(false)
                {
                    for session in sessions {
                        self.items.push(TreeItem::Session {
                            session: session.clone(),
                            project: None,
                        });
                    }
                }
            }
        }
    }

    pub fn toggle_project(&mut self, index: usize) {
        if index < self.items.len() {
            if let TreeItem::Project { name, .. } = &self.items[index] {
                let is_collapsed = self.collapsed_projects.get(name).copied().unwrap_or(false);
                self.collapsed_projects.insert(name.clone(), !is_collapsed);
            }
        }
    }

    pub fn get_parent_project(&self, index: usize) -> Option<String> {
        if index >= self.items.len() {
            return None;
        }

        match &self.items[index] {
            TreeItem::Project { name, .. } => {
                if name == "(no project)" {
                    None
                } else {
                    Some(name.clone())
                }
            }
            TreeItem::Session { project, .. } => project.clone(),
        }
    }
}
