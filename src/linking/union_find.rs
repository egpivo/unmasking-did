use std::collections::HashMap;
use std::hash::Hash;

#[derive(Debug, Clone)]
pub struct UnionFind<T: Eq + Hash + Clone> {
    parent: Vec<usize>,
    rank: Vec<u32>,
    index: HashMap<T, usize>,
    items: Vec<T>,
}

impl<T: Eq + Hash + Clone> Default for UnionFind<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Eq + Hash + Clone> UnionFind<T> {
    pub fn new() -> Self {
        Self {
            parent: Vec::new(),
            rank: Vec::new(),
            index: HashMap::new(),
            items: Vec::new(),
        }
    }

    pub fn add(&mut self, x: T) -> usize {
        if let Some(&i) = self.index.get(&x) {
            return i;
        }
        let i = self.parent.len();
        self.parent.push(i);
        self.rank.push(0);
        self.index.insert(x.clone(), i);
        self.items.push(x);
        i
    }

    pub fn find(&mut self, x: &T) -> Option<T> {
        let i = *self.index.get(x)?;
        let root = self.find_root(i);
        Some(self.items[root].clone())
    }

    pub fn union(&mut self, a: &T, b: &T) -> bool {
        let ai = self.add(a.clone());
        let bi = self.add(b.clone());
        let ra = self.find_root(ai);
        let rb = self.find_root(bi);
        if ra == rb {
            return false;
        }
        let (small, large) = if self.rank[ra] < self.rank[rb] {
            (ra, rb)
        } else {
            (rb, ra)
        };
        self.parent[small] = large;
        if self.rank[small] == self.rank[large] {
            self.rank[large] += 1;
        }
        true
    }

    pub fn components(&mut self) -> Vec<Vec<T>> {
        let mut by_root: HashMap<usize, Vec<T>> = HashMap::new();
        for i in 0..self.items.len() {
            let root = self.find_root(i);
            by_root.entry(root).or_default().push(self.items[i].clone());
        }
        by_root.into_values().collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn find_root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_merges_two_chains() {
        let mut uf: UnionFind<&'static str> = UnionFind::new();
        uf.union(&"a", &"b");
        uf.union(&"c", &"d");
        uf.union(&"b", &"c");
        let comps = uf.components();
        assert_eq!(comps.len(), 1);
        assert_eq!(comps[0].len(), 4);
    }

    #[test]
    fn isolated_nodes_stay_isolated() {
        let mut uf: UnionFind<&'static str> = UnionFind::new();
        uf.add("a");
        uf.add("b");
        let comps = uf.components();
        assert_eq!(comps.len(), 2);
    }
}
