//! Workspace layout model: a binary tree of splits whose leaves are tab
//! groups (Obsidian's "tab groups" / panes). Pure data, no egui.
//!
//! Tabs themselves live in [`crate::state::AppState::tabs`]; the layout only
//! stores their ids. Invariants maintained by every operation:
//! - there is always at least one group and `focused` names an existing one;
//! - every group appears exactly once in the tree;
//! - a tab id appears in at most one group;
//! - an empty group is removed unless it is the only one.

use std::collections::BTreeSet;

use crate::state::TabId;

pub type GroupId = u64;

/// Direction of a new split, Obsidian-style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitDir {
    /// New group to the right ("Split right", `workspace:split-vertical`).
    Right,
    /// New group below ("Split down", `workspace:split-horizontal`).
    Down,
    /// New group to the left (drop on the left edge).
    Left,
    /// New group above (drop on the top edge).
    Up,
}

impl SplitDir {
    fn row(self) -> bool {
        matches!(self, SplitDir::Right | SplitDir::Left)
    }
    fn new_first(self) -> bool {
        matches!(self, SplitDir::Left | SplitDir::Up)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Leaf(GroupId),
    Split {
        /// Children side by side (`true`) or stacked (`false`).
        row: bool,
        /// Share of the first child, 0..1.
        ratio: f32,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    pub fn leaves(&self) -> Vec<GroupId> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect(&self, out: &mut Vec<GroupId>) {
        match self {
            Node::Leaf(g) => out.push(*g),
            Node::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    /// Node reached by following `path` (`false` = first child).
    pub fn at_path_mut(&mut self, path: &[bool]) -> Option<&mut Node> {
        let Some((head, rest)) = path.split_first() else {
            return Some(self);
        };
        match self {
            Node::Leaf(_) => None,
            Node::Split { first, second, .. } => {
                if *head {
                    second.at_path_mut(rest)
                } else {
                    first.at_path_mut(rest)
                }
            }
        }
    }

    /// Replace leaf `target` by `f(leaf)`. Returns true if found.
    fn replace_leaf(&mut self, target: GroupId, f: &mut dyn FnMut(Node) -> Node) -> bool {
        match self {
            Node::Leaf(g) if *g == target => {
                let old = std::mem::replace(self, Node::Leaf(target));
                *self = f(old);
                true
            }
            Node::Leaf(_) => false,
            Node::Split { first, second, .. } => {
                first.replace_leaf(target, f) || second.replace_leaf(target, f)
            }
        }
    }

    /// Remove leaf `target`, replacing its parent split by the sibling.
    /// Returns false if the node is the leaf itself (cannot remove root).
    fn remove_leaf(&mut self, target: GroupId) -> bool {
        let Node::Split { first, second, .. } = self else {
            return false;
        };
        if **first == Node::Leaf(target) {
            let sib = std::mem::replace(&mut **second, Node::Leaf(0));
            *self = sib;
            return true;
        }
        if **second == Node::Leaf(target) {
            let sib = std::mem::replace(&mut **first, Node::Leaf(0));
            *self = sib;
            return true;
        }
        first.remove_leaf(target) || second.remove_leaf(target)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Group {
    pub id: GroupId,
    /// Tabs in display order.
    pub tabs: Vec<TabId>,
    pub active: Option<TabId>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub root: Node,
    pub groups: Vec<Group>,
    pub focused: GroupId,
    pub pinned: BTreeSet<TabId>,
    next_id: GroupId,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            root: Node::Leaf(1),
            groups: vec![Group {
                id: 1,
                ..Group::default()
            }],
            focused: 1,
            pinned: BTreeSet::new(),
            next_id: 2,
        }
    }
}

impl Layout {
    pub fn group(&self, id: GroupId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == id)
    }

    fn group_mut(&mut self, id: GroupId) -> Option<&mut Group> {
        self.groups.iter_mut().find(|g| g.id == id)
    }

    /// Groups in tree order (left to right, top to bottom).
    pub fn ordered_groups(&self) -> Vec<GroupId> {
        self.root.leaves()
    }

    pub fn group_of(&self, tab: TabId) -> Option<GroupId> {
        self.groups
            .iter()
            .find(|g| g.tabs.contains(&tab))
            .map(|g| g.id)
    }

    /// Active tab of the focused group.
    pub fn focused_active(&self) -> Option<TabId> {
        self.group(self.focused).and_then(|g| g.active)
    }

    pub fn is_pinned(&self, tab: TabId) -> bool {
        self.pinned.contains(&tab)
    }

    pub fn toggle_pin(&mut self, tab: TabId) {
        if !self.pinned.remove(&tab) {
            self.pinned.insert(tab);
        }
    }

    /// Insert `tab` into `group` at `index` (default: after the group's
    /// active tab) and make it the active tab.
    pub fn insert_tab(&mut self, group: GroupId, tab: TabId, index: Option<usize>) {
        self.detach(tab);
        let group = if self.group(group).is_some() {
            group
        } else {
            self.focused
        };
        let Some(g) = self.group_mut(group) else {
            return;
        };
        let at = index
            .unwrap_or_else(|| {
                g.active
                    .and_then(|a| g.tabs.iter().position(|t| *t == a))
                    .map_or(g.tabs.len(), |i| i + 1)
            })
            .min(g.tabs.len());
        g.tabs.insert(at, tab);
        g.active = Some(tab);
    }

    /// Focus the group containing `tab` and make it active there.
    pub fn activate(&mut self, tab: TabId) {
        if let Some(gid) = self.group_of(tab) {
            self.focused = gid;
            if let Some(g) = self.group_mut(gid) {
                g.active = Some(tab);
            }
        }
    }

    pub fn focus_group(&mut self, group: GroupId) {
        if self.group(group).is_some() {
            self.focused = group;
        }
    }

    /// Remove `tab` from its group without collapsing anything.
    fn detach(&mut self, tab: TabId) -> Option<GroupId> {
        let gid = self.group_of(tab)?;
        let g = self.group_mut(gid)?;
        let i = g.tabs.iter().position(|t| *t == tab)?;
        g.tabs.remove(i);
        if g.active == Some(tab) {
            g.active = if g.tabs.is_empty() {
                None
            } else {
                Some(g.tabs[i.min(g.tabs.len() - 1)])
            };
        }
        Some(gid)
    }

    /// Close `tab`: removes it and collapses its group if it became empty.
    pub fn remove_tab(&mut self, tab: TabId) {
        self.pinned.remove(&tab);
        if let Some(gid) = self.detach(tab) {
            self.collapse_if_empty(gid);
        }
    }

    /// Remove an empty group from the tree (never the last one).
    pub fn collapse_if_empty(&mut self, gid: GroupId) {
        let empty = self.group(gid).is_some_and(|g| g.tabs.is_empty());
        if !empty || self.groups.len() <= 1 {
            return;
        }
        let order = self.ordered_groups();
        let pos = order.iter().position(|g| *g == gid).unwrap_or(0);
        if !self.root.remove_leaf(gid) {
            return;
        }
        self.groups.retain(|g| g.id != gid);
        if self.focused == gid {
            let remaining = self.ordered_groups();
            self.focused = remaining
                .get(pos.saturating_sub(1))
                .or_else(|| remaining.first())
                .copied()
                .unwrap_or(1);
        }
    }

    /// Split `group`, creating a new empty group next to it. Returns the
    /// new group's id (focused).
    pub fn split(&mut self, group: GroupId, dir: SplitDir) -> GroupId {
        let group = if self.group(group).is_some() {
            group
        } else {
            self.focused
        };
        let new = self.next_id;
        self.next_id += 1;
        let found = self.root.replace_leaf(group, &mut |old| {
            let (first, second) = if dir.new_first() {
                (Node::Leaf(new), old)
            } else {
                (old, Node::Leaf(new))
            };
            Node::Split {
                row: dir.row(),
                ratio: 0.5,
                first: Box::new(first),
                second: Box::new(second),
            }
        });
        if !found {
            return group;
        }
        self.groups.push(Group {
            id: new,
            ..Group::default()
        });
        self.focused = new;
        new
    }

    /// Move `tab` to `to` at `index` (end by default); the source group is
    /// collapsed if it became empty. Focuses the destination.
    pub fn move_tab(&mut self, tab: TabId, to: GroupId, index: Option<usize>) {
        if self.group(to).is_none() {
            return;
        }
        let Some(from) = self.group_of(tab) else {
            return;
        };
        let mut index = index;
        if from == to
            && let (Some(g), Some(i)) = (self.group(to), index)
            && let Some(cur) = self
                .group(to)
                .and_then(|g| g.tabs.iter().position(|t| *t == tab))
            && cur < i
        {
            // Removing the tab first shifts the target index.
            index = Some((i - 1).min(g.tabs.len()));
        }
        self.detach(tab);
        let Some(g) = self.group_mut(to) else {
            return;
        };
        let at = index.unwrap_or(g.tabs.len()).min(g.tabs.len());
        g.tabs.insert(at, tab);
        g.active = Some(tab);
        self.focused = to;
        if from != to {
            self.collapse_if_empty(from);
        }
    }

    /// Move `tab` into a new group created by splitting `target`.
    pub fn move_to_split(&mut self, tab: TabId, target: GroupId, dir: SplitDir) {
        let alone = self.group_of(tab) == Some(target)
            && self.group(target).is_some_and(|g| g.tabs.len() == 1);
        if alone {
            return;
        }
        let new = self.split(target, dir);
        self.move_tab(tab, new, None);
    }

    /// Next/previous tab in the focused group.
    pub fn cycle(&self, delta: isize) -> Option<TabId> {
        let g = self.group(self.focused)?;
        if g.tabs.is_empty() {
            return None;
        }
        let n = g.tabs.len() as isize;
        let cur = g
            .active
            .and_then(|a| g.tabs.iter().position(|t| *t == a))
            .unwrap_or(0) as isize;
        g.tabs.get((cur + delta).rem_euclid(n) as usize).copied()
    }

    /// Focus the next group in tree order (wrapping).
    pub fn cycle_group(&mut self, delta: isize) {
        let order = self.ordered_groups();
        let n = order.len() as isize;
        if n == 0 {
            return;
        }
        let cur = order.iter().position(|g| *g == self.focused).unwrap_or(0) as isize;
        if let Some(g) = order.get((cur + delta).rem_euclid(n) as usize) {
            self.focused = *g;
        }
    }

    /// Drop tab ids that are not in `alive` and collapse empty groups.
    pub fn retain_tabs(&mut self, alive: &[TabId]) {
        for g in &mut self.groups {
            g.tabs.retain(|t| alive.contains(t));
            if g.active.is_some_and(|a| !g.tabs.contains(&a)) {
                g.active = g.tabs.first().copied();
            }
        }
        self.pinned.retain(|t| alive.contains(t));
        let ids: Vec<GroupId> = self.groups.iter().map(|g| g.id).collect();
        for id in ids {
            self.collapse_if_empty(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_tabs(n: u64) -> Layout {
        let mut l = Layout::default();
        for t in 1..=n {
            l.insert_tab(l.focused, t, None);
        }
        l
    }

    #[test]
    fn insert_after_active_and_cycle() {
        let mut l = with_tabs(3);
        assert_eq!(l.group(1).unwrap().tabs, vec![1, 2, 3]);
        l.activate(1);
        l.insert_tab(1, 9, None);
        assert_eq!(l.group(1).unwrap().tabs, vec![1, 9, 2, 3]);
        assert_eq!(l.focused_active(), Some(9));
        assert_eq!(l.cycle(1), Some(2));
        assert_eq!(l.cycle(-2), Some(3));
    }

    #[test]
    fn split_and_move_tab() {
        let mut l = with_tabs(2);
        let g2 = l.split(1, SplitDir::Right);
        assert_eq!(l.focused, g2);
        assert_eq!(l.ordered_groups(), vec![1, g2]);
        l.move_tab(2, g2, None);
        assert_eq!(l.group(1).unwrap().tabs, vec![1]);
        assert_eq!(l.group(g2).unwrap().tabs, vec![2]);
        assert_eq!(l.focused_active(), Some(2));
        assert!(matches!(l.root, Node::Split { row: true, .. }));
        // Split down the second group, new group goes below.
        let g3 = l.split(g2, SplitDir::Down);
        l.move_tab(1, g3, None);
        // Group 1 became empty and was removed.
        assert!(l.group(1).is_none());
        assert_eq!(l.ordered_groups(), vec![g2, g3]);
        assert!(matches!(l.root, Node::Split { row: false, .. }));
    }

    #[test]
    fn closing_last_tab_merges_group() {
        let mut l = with_tabs(1);
        let g2 = l.split(1, SplitDir::Right);
        l.insert_tab(g2, 5, None);
        l.activate(5);
        l.remove_tab(5);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.focused, 1);
        assert_eq!(l.root, Node::Leaf(1));
        // The last group is never removed.
        l.remove_tab(1);
        assert_eq!(l.groups.len(), 1);
        assert_eq!(l.focused_active(), None);
    }

    #[test]
    fn move_within_group_reorders() {
        let mut l = with_tabs(4);
        l.move_tab(1, 1, Some(3));
        assert_eq!(l.group(1).unwrap().tabs, vec![2, 3, 1, 4]);
        l.move_tab(4, 1, Some(0));
        assert_eq!(l.group(1).unwrap().tabs, vec![4, 2, 3, 1]);
    }

    #[test]
    fn move_to_edge_split() {
        let mut l = with_tabs(2);
        l.move_to_split(2, 1, SplitDir::Left);
        let order = l.ordered_groups();
        assert_eq!(order.len(), 2);
        assert_eq!(l.group(order[0]).unwrap().tabs, vec![2]);
        // Splitting a group with its only tab is a no-op.
        l.move_to_split(2, order[0], SplitDir::Up);
        assert_eq!(l.groups.len(), 2);
    }

    #[test]
    fn nested_removal_keeps_tree_consistent() {
        let mut l = with_tabs(1);
        let g2 = l.split(1, SplitDir::Right);
        l.insert_tab(g2, 2, None);
        let g3 = l.split(g2, SplitDir::Down);
        l.insert_tab(g3, 3, None);
        l.remove_tab(2);
        assert_eq!(l.ordered_groups(), vec![1, g3]);
        l.retain_tabs(&[1]);
        assert_eq!(l.ordered_groups(), vec![1]);
        assert!(l.group(l.focused).is_some());
    }

    #[test]
    fn pins_and_path_access() {
        let mut l = with_tabs(2);
        l.toggle_pin(1);
        assert!(l.is_pinned(1));
        l.toggle_pin(1);
        assert!(!l.is_pinned(1));
        l.split(1, SplitDir::Down);
        if let Some(Node::Split { ratio, .. }) = l.root.at_path_mut(&[]) {
            *ratio = 0.3;
        }
        assert!(matches!(l.root, Node::Split { ratio, .. } if (ratio - 0.3).abs() < 1e-6));
        assert!(l.root.at_path_mut(&[true]).is_some());
        assert!(l.root.at_path_mut(&[true, false]).is_none());
        l.cycle_group(1);
        assert_eq!(l.focused, 1);
    }
}
