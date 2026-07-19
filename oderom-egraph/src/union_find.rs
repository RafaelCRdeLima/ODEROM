//! Standard disjoint-set union-find, path-halved on `find`. No union-by-
//! rank: at the group sizes this project's e-graphs reach (dozens of
//! e-classes, not millions), the asymptotic difference is unmeasurable
//! and rank bookkeeping is one more thing to get subtly wrong.

#[derive(Clone, Debug, Default)]
pub struct UnionFind {
    parent: Vec<u32>,
}

impl UnionFind {
    /// Adds a new singleton set, returning its id.
    pub fn make_set(&mut self) -> u32 {
        let id = self.parent.len() as u32;
        self.parent.push(id);
        id
    }

    /// The representative of the set containing `x`.
    pub fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            let grandparent = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = grandparent; // path halving
            x = self.parent[x as usize];
        }
        x
    }

    /// Merges the sets containing `a` and `b`; a no-op if already merged.
    /// Returns the resulting representative.
    pub fn union(&mut self, a: u32, b: u32) -> u32 {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[ra as usize] = rb;
        }
        rb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_sets_are_distinct() {
        let mut uf = UnionFind::default();
        let a = uf.make_set();
        let b = uf.make_set();
        assert_ne!(uf.find(a), uf.find(b));
    }

    #[test]
    fn union_makes_two_sets_equal() {
        let mut uf = UnionFind::default();
        let a = uf.make_set();
        let b = uf.make_set();
        uf.union(a, b);
        assert_eq!(uf.find(a), uf.find(b));
    }

    #[test]
    fn union_is_transitive_through_a_chain() {
        let mut uf = UnionFind::default();
        let a = uf.make_set();
        let b = uf.make_set();
        let c = uf.make_set();
        uf.union(a, b);
        uf.union(b, c);
        assert_eq!(uf.find(a), uf.find(c));
    }

    #[test]
    fn unioning_an_already_merged_pair_is_a_no_op() {
        let mut uf = UnionFind::default();
        let a = uf.make_set();
        let b = uf.make_set();
        uf.union(a, b);
        let root_before = uf.find(a);
        uf.union(a, b);
        assert_eq!(uf.find(a), root_before);
    }

    #[test]
    fn unrelated_sets_stay_distinct_after_other_unions() {
        let mut uf = UnionFind::default();
        let a = uf.make_set();
        let b = uf.make_set();
        let c = uf.make_set();
        let d = uf.make_set();
        uf.union(a, b);
        assert_ne!(uf.find(c), uf.find(d));
        assert_ne!(uf.find(a), uf.find(c));
    }
}
