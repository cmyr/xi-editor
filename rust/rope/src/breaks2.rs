// Copyright 2016 The xi-editor Authors.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A module for representing a set of breaks, typically used for
//! storing the result of line breaking.

use crate::interval::Interval;
use crate::tree::{DefaultMetric, Leaf, Metric, Node, NodeInfo, TreeBuilder};
use std::cmp::min;
use std::mem;

/// A set of indexes. A motivating use is storing line breaks.
pub type Breaks = Node<BreaksInfo>;

const MIN_LEAF: usize = 32;
const MAX_LEAF: usize = 64;

type Offset = usize;
type Width = usize;

// Here the base units are arbitrary, but most commonly match the base units
// of the rope storing the underlying string.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BreaksLeaf {
    /// Length, in base units.
    len: usize,
    /// Indexes, represent as offsets from the start of the leaf.
    data: Vec<(Offset, Width)>,
}

/// The number of breaks.
#[derive(Clone, Debug)]
pub struct BreaksInfo {
    count: usize,
    max_width: usize,
}

impl Leaf for BreaksLeaf {
    fn len(&self) -> usize {
        self.len
    }

    fn is_ok_child(&self) -> bool {
        self.data.len() >= MIN_LEAF
    }

    fn push_maybe_split(&mut self, other: &BreaksLeaf, iv: Interval) -> Option<BreaksLeaf> {
        //eprintln!("push_maybe_split {:?} {:?} {}", self, other, iv);
        let (start, end) = iv.start_end();
        for &(v, w) in &other.data {
            if start < v && v <= end {
                self.data.push((v - start + self.len, w));
            }
        }
        // the min with other.len() shouldn't be needed
        self.len += min(end, other.len()) - start;

        if self.data.len() <= MAX_LEAF {
            None
        } else {
            let splitpoint = self.data.len() / 2; // number of breaks
            let splitpoint_units = self.data[splitpoint - 1].0;

            let mut new = self.data.split_off(splitpoint);
            for (x, _) in &mut new {
                *x -= splitpoint_units;
            }

            let new_len = self.len - splitpoint_units;
            self.len = splitpoint_units;
            Some(BreaksLeaf { len: new_len, data: new })
        }
    }
}

impl NodeInfo for BreaksInfo {
    type L = BreaksLeaf;

    fn accumulate(&mut self, other: &Self) {
        self.count += other.count;
        self.max_width = other.max_width.max(self.max_width);
    }

    fn compute_info(l: &BreaksLeaf) -> BreaksInfo {
        let count = l.data.len();
        let max_width = l.data.iter().map(|(_, w)| *w).max().unwrap_or(0);
        BreaksInfo { count, max_width }
    }
}

impl DefaultMetric for BreaksInfo {
    type DefaultMetric = BreaksBaseMetric;
}

impl BreaksLeaf {
    /// Exposed for testing.
    #[doc(hidden)]
    pub fn get_data_cloned(&self) -> Vec<(Offset, Width)> {
        self.data.clone()
    }
}

#[derive(Copy, Clone)]
pub struct BreaksMetric(());

impl Metric<BreaksInfo> for BreaksMetric {
    fn measure(info: &BreaksInfo, _: usize) -> usize {
        info.count
    }

    fn to_base_units(l: &BreaksLeaf, in_measured_units: usize) -> usize {
        if in_measured_units > l.data.len() {
            l.len + 1
        } else if in_measured_units == 0 {
            0
        } else {
            l.data[in_measured_units - 1].0
        }
    }

    fn from_base_units(l: &BreaksLeaf, in_base_units: usize) -> usize {
        match l.data.binary_search_by_key(&in_base_units, |&(n, _)| n) {
            Ok(n) => n + 1,
            Err(n) => n,
        }
    }

    fn is_boundary(l: &BreaksLeaf, offset: usize) -> bool {
        l.data.binary_search_by_key(&offset, |&(n, _)| n).is_ok()
    }

    fn prev(l: &BreaksLeaf, offset: usize) -> Option<usize> {
        for i in 0..l.data.len() {
            if offset <= l.data[i].0 {
                if i == 0 {
                    return None;
                } else {
                    return Some(l.data[i - 1].0);
                }
            }
        }
        l.data.last().map(|(n, _)| *n)
    }

    fn next(l: &BreaksLeaf, offset: usize) -> Option<usize> {
        let n = match l.data.binary_search_by_key(&offset, |&(n, _)| n) {
            Ok(n) => n + 1,
            Err(n) => n,
        };

        if n == l.data.len() {
            None
        } else {
            Some(l.data[n].0)
        }
    }

    fn can_fragment() -> bool {
        true
    }
}

#[derive(Copy, Clone)]
pub struct BreaksBaseMetric(());

impl Metric<BreaksInfo> for BreaksBaseMetric {
    fn measure(_: &BreaksInfo, len: usize) -> usize {
        len
    }

    fn to_base_units(_: &BreaksLeaf, in_measured_units: usize) -> usize {
        in_measured_units
    }

    fn from_base_units(_: &BreaksLeaf, in_base_units: usize) -> usize {
        in_base_units
    }

    fn is_boundary(l: &BreaksLeaf, offset: usize) -> bool {
        BreaksMetric::is_boundary(l, offset)
    }

    fn prev(l: &BreaksLeaf, offset: usize) -> Option<usize> {
        BreaksMetric::prev(l, offset)
    }

    fn next(l: &BreaksLeaf, offset: usize) -> Option<usize> {
        BreaksMetric::next(l, offset)
    }

    fn can_fragment() -> bool {
        true
    }
}

// Additional functions specific to breaks

impl Breaks {
    // a length with no break, useful in edit operations; for
    // other use cases, use the builder.
    pub fn new_no_break(len: usize) -> Breaks {
        let leaf = BreaksLeaf { len, data: vec![] };
        Node::from_leaf(leaf)
    }

    pub fn max_width(&self) -> Width {
        self.get_info().max_width
    }
}

pub struct BreakBuilder {
    b: TreeBuilder<BreaksInfo>,
    leaf: BreaksLeaf,
}

impl Default for BreakBuilder {
    fn default() -> BreakBuilder {
        BreakBuilder { b: TreeBuilder::new(), leaf: BreaksLeaf::default() }
    }
}

impl BreakBuilder {
    pub fn new() -> BreakBuilder {
        BreakBuilder::default()
    }

    pub fn add_break(&mut self, len: usize, width: Width) {
        if self.leaf.data.len() == MAX_LEAF {
            let leaf = mem::replace(&mut self.leaf, BreaksLeaf::default());
            self.b.push(Node::from_leaf(leaf));
        }
        self.leaf.len += len;
        self.leaf.data.push((self.leaf.len, width));
    }

    pub fn add_no_break(&mut self, len: usize) {
        self.leaf.len += len;
    }

    pub fn build(mut self) -> Breaks {
        self.b.push(Node::from_leaf(self.leaf));
        self.b.build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_largest_line() {
        let mut b = BreakBuilder::new();
        b.add_break(4, 4);
        b.add_break(4, 10);
        b.add_break(4, 2);
        let mut breaks = b.build();

        assert_eq!(breaks.len(), 4 * 3);
        assert_eq!(breaks.max_width(), 10);

        breaks.edit(4..8, Breaks::default());
        assert_eq!(breaks.len(), 4 * 2);
        assert_eq!(breaks.max_width(), 4);
    }
}
