use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ignore::WalkBuilder;

const MAX_NODES: usize = 40_000;
const MAX_DEPTH: usize = 32;

struct Node {
    name: String,
    path: PathBuf,
    is_dir: bool,
    children: Vec<Node>,
}

#[derive(Clone)]
pub struct Row {
    pub name: String,
    pub path: PathBuf,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
}

pub struct FsTree {
    pub root: PathBuf,
    nodes: Vec<Node>,
    pub rows: Vec<Row>,
    pub expanded: HashSet<PathBuf>,
    pub show_hidden: bool,
    pub selected: usize,
    pub total: usize,
    pub truncated: bool,
}

impl FsTree {
    pub fn new(root: PathBuf, show_hidden: bool) -> Self {
        let mut tree = Self {
            root,
            nodes: Vec::new(),
            rows: Vec::new(),
            expanded: HashSet::new(),
            show_hidden,
            selected: 0,
            total: 0,
            truncated: false,
        };
        tree.rebuild();
        tree
    }

    pub fn rebuild(&mut self) {
        let mut by_parent: HashMap<PathBuf, Vec<Node>> = HashMap::new();
        let mut total = 0usize;
        let mut truncated = false;

        let mut builder = WalkBuilder::new(&self.root);
        builder
            .hidden(!self.show_hidden)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .require_git(false)
            .follow_links(false)
            .max_depth(Some(MAX_DEPTH))
            .filter_entry(|entry| entry.file_name() != ".git");

        for entry in builder.build().filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path == self.root {
                continue;
            }
            total += 1;
            if total > MAX_NODES {
                truncated = true;
                break;
            }
            let Some(parent) = path.parent() else {
                continue;
            };
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            by_parent.entry(parent.to_path_buf()).or_default().push(Node {
                name: entry.file_name().to_string_lossy().to_string(),
                path: path.to_path_buf(),
                is_dir,
                children: Vec::new(),
            });
        }

        fn assemble(parent: &Path, by_parent: &mut HashMap<PathBuf, Vec<Node>>) -> Vec<Node> {
            let mut nodes = by_parent.remove(parent).unwrap_or_default();
            nodes.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                    .then_with(|| a.name.cmp(&b.name))
            });
            for node in &mut nodes {
                if node.is_dir {
                    node.children = assemble(&node.path, by_parent);
                }
            }
            nodes
        }

        self.nodes = assemble(&self.root, &mut by_parent);
        self.total = total;
        self.truncated = truncated;
        self.rebuild_rows();
    }

    fn rebuild_rows(&mut self) {
        fn walk(nodes: &[Node], depth: usize, expanded: &HashSet<PathBuf>, out: &mut Vec<Row>) {
            for node in nodes {
                let is_expanded = node.is_dir && expanded.contains(&node.path);
                out.push(Row {
                    name: node.name.clone(),
                    path: node.path.clone(),
                    depth,
                    is_dir: node.is_dir,
                    expanded: is_expanded,
                });
                if is_expanded {
                    walk(&node.children, depth + 1, expanded, out);
                }
            }
        }

        let mut rows = Vec::new();
        walk(&self.nodes, 0, &self.expanded, &mut rows);
        self.rows = rows;
        self.clamp();
    }

    pub fn clamp(&mut self) {
        if self.rows.is_empty() {
            self.selected = 0;
        } else if self.selected >= self.rows.len() {
            self.selected = self.rows.len() - 1;
        }
    }

    pub fn selected_path(&self) -> Option<&Path> {
        self.rows.get(self.selected).map(|row| row.path.as_path())
    }

    pub fn select_path(&mut self, path: &Path) -> bool {
        if let Some(index) = self.rows.iter().position(|row| row.path == path) {
            self.selected = index;
            true
        } else {
            false
        }
    }

    /// Expand every ancestor directory and select `path`. Used when jumping
    /// from the change list into the file tree.
    pub fn reveal(&mut self, path: &Path) -> bool {
        let mut ancestors = Vec::new();
        let mut current = path.parent();
        while let Some(parent) = current {
            if parent == self.root {
                break;
            }
            ancestors.push(parent.to_path_buf());
            current = parent.parent();
        }

        let mut changed = false;
        for ancestor in ancestors {
            if self.expanded.insert(ancestor) {
                changed = true;
            }
        }
        if changed {
            self.rebuild_rows();
        }
        self.select_path(path)
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        if !self.rows.is_empty() {
            self.selected = (self.selected + 1).min(self.rows.len() - 1);
        }
    }

    pub fn toggle(&mut self) {
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return;
        };
        if !row.is_dir {
            return;
        }
        if !self.expanded.insert(row.path.clone()) {
            self.expanded.remove(&row.path);
        }
        self.rebuild_rows();
        self.select_path(&row.path);
    }

    /// Right arrow: expand a collapsed dir, or step into an expanded one.
    pub fn expand(&mut self) {
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return;
        };
        if !row.is_dir {
            return;
        }
        if !row.expanded {
            self.expanded.insert(row.path.clone());
            self.rebuild_rows();
            self.select_path(&row.path);
        } else if self.selected + 1 < self.rows.len()
            && self.rows[self.selected + 1].depth > row.depth
        {
            self.selected += 1;
        }
    }

    /// Left arrow: collapse an expanded dir, otherwise jump to the parent.
    pub fn left(&mut self) {
        let Some(row) = self.rows.get(self.selected).cloned() else {
            return;
        };
        if row.is_dir && row.expanded {
            self.expanded.remove(&row.path);
            self.rebuild_rows();
            self.select_path(&row.path);
            return;
        }
        if let Some(parent) = row.path.parent() {
            self.select_path(parent);
        }
    }

    pub fn set_hidden(&mut self, show_hidden: bool) {
        if self.show_hidden != show_hidden {
            self.show_hidden = show_hidden;
            let selected = self.selected_path().map(Path::to_path_buf);
            self.rebuild();
            if let Some(path) = selected {
                self.select_path(&path);
            }
        }
    }
}
