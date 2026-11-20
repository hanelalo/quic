use std::path::Path;
use std::process::Command;

#[derive(Default)]
pub struct GitInfo {
    pub is_repo: bool,
    pub branch: Option<String>,
    pub changes: Vec<Change>,
    pub error: Option<String>,
}

pub struct Change {
    pub x: char,
    pub y: char,
    pub path: String,
    pub orig: Option<String>,
}

impl Change {
    /// The status letter that best describes this entry.
    pub fn status(&self) -> char {
        if self.x == '?' && self.y == '?' {
            return '?';
        }
        if self.x != ' ' && self.x != '.' {
            self.x
        } else if self.y != ' ' && self.y != '.' {
            self.y
        } else {
            ' '
        }
    }

}

pub fn load(root: &Path) -> GitInfo {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--branch",
            "--untracked-files=all",
        ])
        .output();

    let output = match output {
        Ok(output) => output,
        Err(err) => {
            return GitInfo {
                error: Some(format!("cannot run git: {err}")),
                ..GitInfo::default()
            };
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = if stderr.contains("not a git repository") {
            "not a git repository".to_string()
        } else {
            stderr
                .lines()
                .next()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| format!("git failed ({})", output.status))
        };
        return GitInfo {
            is_repo: false,
            error: Some(message),
            ..GitInfo::default()
        };
    }

    let mut branch = None;
    let mut changes = Vec::new();
    let mut fields = output.stdout.split(|byte| *byte == 0);

    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let text = String::from_utf8_lossy(field);
        if let Some(rest) = text.strip_prefix("## ") {
            branch = parse_branch(rest);
            continue;
        }
        if field.len() < 3 {
            continue;
        }
        let x = field[0] as char;
        let y = field[1] as char;
        let mut path = String::from_utf8_lossy(&field[3..]).to_string();
        let mut orig = None;
        if (matches!(x, 'R' | 'C') || matches!(y, 'R' | 'C'))
            && let Some(original) = fields.next()
        {
            let original = String::from_utf8_lossy(original).to_string();
            if matches!(x, 'R' | 'C') {
                orig = Some(original);
            } else {
                orig = Some(std::mem::take(&mut path));
                path = original;
            }
        }
        changes.push(Change { x, y, path, orig });
    }

    changes.sort_by(|a, b| a.path.cmp(&b.path));

    GitInfo {
        is_repo: true,
        branch,
        changes,
        error: None,
    }
}

fn parse_branch(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let value = value.strip_prefix("No commits yet on ").unwrap_or(value);
    let value = value.split("...").next().unwrap_or(value);
    let value = value.split(" [").next().unwrap_or(value);
    let value = value.trim();
    if value.is_empty() {
        None
    } else if value.starts_with("HEAD") {
        Some("HEAD".to_string())
    } else {
        Some(value.to_string())
    }
}
