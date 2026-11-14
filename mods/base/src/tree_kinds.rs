use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use weft_sdk::noise::Rng;
use weft_sdk::*;

weft_sdk::abi::data! {
    pub struct TreeKind {
        pub name: String,
        pub weight: u32,
    }
}
weft_sdk::abi::data! {
    pub struct TreeKinds {
        pub entries: Vec<TreeKind>,
    }
}

/// What a biome grows: tree kind names with weights.
pub static TREE_KINDS: Component<TreeKinds> = Component::named("tree_kinds");

impl TreeKinds {
    pub fn new<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = (S, u32)>,
        S: Into<String>,
    {
        Self {
            entries: entries
                .into_iter()
                .map(|(name, weight)| TreeKind { name: name.into(), weight })
                .collect(),
        }
    }
    pub fn get(&self, name: &str) -> Option<u32> {
        self.entries
            .iter()
            .find(|e| e.name == name)
            .map(|e| e.weight)
    }
    pub fn pick(&self, mut roll: u64) -> Option<&str> {
        let total: u64 = self.entries.iter().map(|e| u64::from(e.weight)).sum();
        if total == 0 {
            return None;
        }
        roll %= total;
        for entry in &self.entries {
            let weight = u64::from(entry.weight);
            if roll < weight {
                return Some(&entry.name);
            }
            roll -= weight;
        }
        None
    }
    pub fn iter(&self) -> impl Iterator<Item = &TreeKind> {
        self.entries.iter()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl<const N: usize> From<[(&'static str, u32); N]> for TreeKinds {
    fn from(entries: [(&'static str, u32); N]) -> Self {
        Self::new(entries)
    }
}
impl<S: Into<String>> FromIterator<(S, u32)> for TreeKinds {
    fn from_iter<I: IntoIterator<Item = (S, u32)>>(iter: I) -> Self {
        Self::new(iter)
    }
}
impl<'a> IntoIterator for &'a TreeKinds {
    type Item = &'a TreeKind;
    type IntoIter = std::slice::Iter<'a, TreeKind>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}
pub type TreePlace = fn((i32, i32, i32), &mut Rng, &mut dyn FnMut(i32, i32, i32, u32, bool));
static TREE_DB: LazyLock<RwLock<HashMap<&'static str, TreePlace>>> = LazyLock::new(|| RwLock::new(HashMap::new()));
pub fn register_tree(name: &'static str, place: TreePlace) {
    TREE_DB
        .write()
        .expect("tree registry poisoned")
        .insert(name, place);
}
pub fn tree_place(name: &str) -> Option<TreePlace> {
    TREE_DB.read().ok()?.get(name).copied()
}
