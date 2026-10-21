//! Bookmarks core plugin: `.obsidian/bookmarks.json` in Obsidian's format.
//!
//! ```json
//! {"items":[{"type":"file","ctime":1700000000000,"path":"x.md"},
//!           {"type":"file","ctime":1,"path":"x.md","subpath":"#Heading"},
//!           {"type":"folder","ctime":1,"path":"dir"},
//!           {"type":"search","ctime":1,"query":"tag:#x"},
//!           {"type":"group","ctime":1,"title":"G","items":[...]}]}
//! ```
//! Unknown item types and unknown fields are preserved verbatim.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const BOOKMARKS_JSON: &str = "bookmarks.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bookmark {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub ctime: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subpath: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<Bookmark>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Bookmark {
    fn new(kind: &str, ctime: i64) -> Self {
        Self {
            kind: kind.to_string(),
            ctime,
            path: None,
            subpath: None,
            query: None,
            title: None,
            items: None,
            extra: Map::new(),
        }
    }

    pub fn file(path: &str, ctime: i64) -> Self {
        Self {
            path: Some(path.to_string()),
            ..Self::new("file", ctime)
        }
    }

    pub fn heading(path: &str, heading: &str, ctime: i64) -> Self {
        Self {
            subpath: Some(format!("#{heading}")),
            ..Self::file(path, ctime)
        }
    }

    pub fn folder(path: &str, ctime: i64) -> Self {
        Self {
            path: Some(path.to_string()),
            ..Self::new("folder", ctime)
        }
    }

    pub fn search(query: &str, ctime: i64) -> Self {
        Self {
            query: Some(query.to_string()),
            ..Self::new("search", ctime)
        }
    }

    pub fn group(title: &str, ctime: i64) -> Self {
        Self {
            title: Some(title.to_string()),
            items: Some(Vec::new()),
            ..Self::new("group", ctime)
        }
    }

    pub fn is_group(&self) -> bool {
        self.kind == "group"
    }

    /// Text shown in the bookmarks pane.
    pub fn label(&self) -> String {
        if let Some(t) = self.title.as_deref().filter(|t| !t.is_empty()) {
            return t.to_string();
        }
        match self.kind.as_str() {
            "file" => {
                let p = self.path.as_deref().unwrap_or("");
                let stem = p
                    .rsplit('/')
                    .next()
                    .unwrap_or(p)
                    .trim_end_matches(".md")
                    .to_string();
                match self.subpath.as_deref() {
                    Some(s) => format!("{stem} › {}", s.trim_start_matches('#')),
                    None => stem,
                }
            }
            "folder" => self.path.clone().unwrap_or_default(),
            "search" => self.query.clone().unwrap_or_default(),
            other => other.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Bookmarks {
    #[serde(default)]
    pub items: Vec<Bookmark>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Bookmarks {
    pub fn from_json(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{\"items\":[]}".into())
    }

    fn visit<'a>(items: &'a [Bookmark], f: &mut dyn FnMut(&'a Bookmark) -> bool) -> bool {
        for b in items {
            if f(b) {
                return true;
            }
            if let Some(children) = &b.items
                && Self::visit(children, f)
            {
                return true;
            }
        }
        false
    }

    fn visit_mut(items: &mut [Bookmark], f: &mut dyn FnMut(&mut Bookmark)) {
        for b in items {
            f(b);
            if let Some(children) = &mut b.items {
                Self::visit_mut(children, f);
            }
        }
    }

    /// Is `path` bookmarked as a whole file (anywhere, also in groups)?
    pub fn has_file(&self, path: &str) -> bool {
        Self::visit(&self.items, &mut |b| {
            b.kind == "file" && b.subpath.is_none() && b.path.as_deref() == Some(path)
        })
    }

    pub fn contains(&self, item: &Bookmark) -> bool {
        Self::visit(&self.items, &mut |b| {
            b.kind == item.kind
                && b.path == item.path
                && b.subpath == item.subpath
                && b.query == item.query
                && !b.is_group()
        })
    }

    /// Add an item at the top level (no duplicates). Returns false if it
    /// was already there.
    pub fn add(&mut self, item: Bookmark) -> bool {
        if !item.is_group() && self.contains(&item) {
            return false;
        }
        self.items.push(item);
        true
    }

    fn list_mut(&mut self, parent: &[usize]) -> Option<&mut Vec<Bookmark>> {
        let mut list = &mut self.items;
        for &i in parent {
            list = list.get_mut(i)?.items.as_mut()?;
        }
        Some(list)
    }

    pub fn get(&self, path: &[usize]) -> Option<&Bookmark> {
        let (last, parent) = path.split_last()?;
        let mut list = &self.items;
        for &i in parent {
            list = list.get(i)?.items.as_ref()?;
        }
        list.get(*last)
    }

    pub fn remove(&mut self, path: &[usize]) -> Option<Bookmark> {
        let (last, parent) = path.split_last()?;
        let list = self.list_mut(parent)?;
        (*last < list.len()).then(|| list.remove(*last))
    }

    /// Remove the whole-file bookmark of `path` (toggle).
    pub fn remove_file(&mut self, path: &str) {
        fn rec(items: &mut Vec<Bookmark>, path: &str) {
            items.retain(|b| {
                !(b.kind == "file" && b.subpath.is_none() && b.path.as_deref() == Some(path))
            });
            for b in items {
                if let Some(c) = &mut b.items {
                    rec(c, path);
                }
            }
        }
        rec(&mut self.items, path);
    }

    /// Move an item one step up (`-1`) or down (`1`) in its list.
    pub fn shift(&mut self, path: &[usize], delta: isize) -> bool {
        let Some((last, parent)) = path.split_last() else {
            return false;
        };
        let Some(list) = self.list_mut(parent) else {
            return false;
        };
        let to = *last as isize + delta;
        if to < 0 || to as usize >= list.len() || *last >= list.len() {
            return false;
        }
        list.swap(*last, to as usize);
        true
    }

    /// Move the item at `from` into the group at `group` (or the top level
    /// when `group` is empty), at the end.
    pub fn move_into(&mut self, from: &[usize], group: &[usize]) -> bool {
        if group.starts_with(from) {
            return false; // into itself
        }
        let target_ok = group.is_empty() || self.get(group).is_some_and(Bookmark::is_group);
        if !target_ok {
            return false;
        }
        // Removing `from` shifts later siblings: adjust the group path.
        let mut group = group.to_vec();
        if from.len() <= group.len() {
            let depth = from.len() - 1;
            if group[..depth] == from[..depth] && group[depth] > from[depth] {
                group[depth] -= 1;
            }
        }
        let Some(item) = self.remove(from) else {
            return false;
        };
        match self.list_mut(&group) {
            Some(list) => {
                list.push(item);
                true
            }
            None => {
                self.items.push(item);
                true
            }
        }
    }

    /// Set the title of the item at `path`. An empty title clears it (not
    /// for groups, which keep their previous title).
    pub fn rename_item(&mut self, path: &[usize], title: &str) -> bool {
        let Some((last, parent)) = path.split_last() else {
            return false;
        };
        let Some(item) = self.list_mut(parent).and_then(|l| l.get_mut(*last)) else {
            return false;
        };
        let title = title.trim();
        if title.is_empty() {
            if item.is_group() {
                return false;
            }
            item.title = None;
        } else {
            item.title = Some(title.to_string());
        }
        true
    }

    /// Delete the group at `path`; its items take its place in the parent.
    pub fn ungroup(&mut self, path: &[usize]) -> bool {
        let Some((last, parent)) = path.split_last() else {
            return false;
        };
        let Some(list) = self.list_mut(parent) else {
            return false;
        };
        if !list.get(*last).is_some_and(Bookmark::is_group) {
            return false;
        }
        let group = list.remove(*last);
        let children = group.items.unwrap_or_default();
        for (k, c) in children.into_iter().enumerate() {
            list.insert(*last + k, c);
        }
        true
    }

    /// Update paths after a file rename.
    pub fn rename_file(&mut self, from: &str, to: &str) {
        Self::visit_mut(&mut self.items, &mut |b| {
            if b.kind == "file" && b.path.as_deref() == Some(from) {
                b.path = Some(to.to_string());
            }
        });
    }

    /// Update paths after a folder rename/move.
    pub fn rename_folder(&mut self, from: &str, to: &str) {
        let prefix = format!("{from}/");
        Self::visit_mut(&mut self.items, &mut |b| {
            let Some(p) = b.path.clone() else { return };
            if b.kind == "folder" && p == from {
                b.path = Some(to.to_string());
            } else if let Some(rest) = p.strip_prefix(&prefix) {
                b.path = Some(format!("{to}/{rest}"));
            }
        });
    }
}

/// Current time in milliseconds since the epoch (bookmark `ctime`).
pub fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OBSIDIAN: &str = r##"{
  "items": [
    {"type": "file", "ctime": 1700000000000, "path": "Accueil.md"},
    {"type": "file", "ctime": 1700000000001, "path": "Projets/Rust notes.md", "subpath": "#Ownership"},
    {"type": "group", "ctime": 1700000000002, "items": [
        {"type": "folder", "ctime": 1700000000003, "path": "Projets"},
        {"type": "search", "ctime": 1700000000004, "query": "tag:#rust"}
      ], "title": "Work"},
    {"type": "graph", "ctime": 1700000000005, "options": {"depth": 2}, "title": "G"}
  ],
  "rev": 3
}"##;

    #[test]
    fn obsidian_round_trip() {
        let b = Bookmarks::from_json(OBSIDIAN);
        assert_eq!(b.items.len(), 4);
        assert!(b.has_file("Accueil.md"));
        assert!(!b.has_file("Projets/Rust notes.md"));
        assert_eq!(b.items[1].label(), "Rust notes › Ownership");
        let group = b.items[2].items.as_ref().unwrap();
        assert_eq!(group[1].query.as_deref(), Some("tag:#rust"));
        // Unknown types and keys survive.
        let json = b.to_json();
        let again: Value = serde_json::from_str(&json).unwrap();
        let orig: Value = serde_json::from_str(OBSIDIAN).unwrap();
        assert_eq!(again, orig);
    }

    #[test]
    fn add_remove_move() {
        let mut b = Bookmarks::default();
        assert!(b.add(Bookmark::file("a.md", 1)));
        assert!(!b.add(Bookmark::file("a.md", 2)));
        b.add(Bookmark::heading("a.md", "H", 3));
        b.add(Bookmark::group("G", 4));
        b.add(Bookmark::search("x", 5));
        assert!(b.move_into(&[0], &[2]));
        // [heading, group[a.md], search]
        assert_eq!(b.items.len(), 3);
        assert!(b.has_file("a.md"));
        assert_eq!(b.get(&[1, 0]).unwrap().path.as_deref(), Some("a.md"));
        assert!(b.shift(&[2], -1));
        assert_eq!(b.items[1].kind, "search");
        assert!(!b.move_into(&[2], &[2, 0]));
        b.remove_file("a.md");
        assert!(!b.has_file("a.md"));
        assert!(b.remove(&[0]).is_some());
        assert_eq!(b.items.len(), 2);
        let json = b.to_json();
        assert!(json.contains("\"items\": []"), "{json}");
    }

    #[test]
    fn rename_and_ungroup() {
        let mut b = Bookmarks::from_json(OBSIDIAN);
        assert!(b.rename_item(&[2], "Travail"));
        assert_eq!(b.items[2].label(), "Travail");
        assert!(!b.rename_item(&[2], "  "), "groups keep a title");
        assert!(b.rename_item(&[2, 0], "Projets!"));
        assert_eq!(b.get(&[2, 0]).unwrap().label(), "Projets!");
        assert!(b.rename_item(&[2, 0], ""));
        assert_eq!(b.get(&[2, 0]).unwrap().label(), "Projets");
        assert!(!b.ungroup(&[0]), "only groups");
        assert!(b.ungroup(&[2]));
        // [file, file#h, folder, search, graph]
        assert_eq!(b.items.len(), 5);
        assert_eq!(b.items[2].kind, "folder");
        assert_eq!(b.items[3].kind, "search");
        assert_eq!(b.items[4].kind, "graph");
        assert!(b.to_json().contains("\"rev\": 3"));
    }

    #[test]
    fn renames_follow_files_and_folders() {
        let mut b = Bookmarks::default();
        b.add(Bookmark::file("dir/a.md", 1));
        b.add(Bookmark::folder("dir", 1));
        b.add(Bookmark::folder("dir/sub", 1));
        b.rename_file("dir/a.md", "dir/b.md");
        assert!(b.has_file("dir/b.md"));
        b.rename_folder("dir", "new");
        assert!(b.has_file("new/b.md"));
        assert_eq!(b.items[1].path.as_deref(), Some("new"));
        assert_eq!(b.items[2].path.as_deref(), Some("new/sub"));
    }

    #[test]
    fn invalid_json_is_empty() {
        assert_eq!(Bookmarks::from_json("nope"), Bookmarks::default());
    }
}
