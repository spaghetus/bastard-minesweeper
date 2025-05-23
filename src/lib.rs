use std::{
    collections::HashMap,
    mem::take,
    ops::{Deref, DerefMut, Index, IndexMut, RangeInclusive},
    sync::Arc,
    time::{Duration, Instant},
};

use itertools::Itertools;
use llist::{ConsRef, LList};
use ndarray::{Array2, ArrayView2, ArrayViewMut2};
use rand::{Rng, distr::slice::Choose, rng, seq::SliceRandom};
use rayon::prelude::*;

#[derive(Clone, Copy, Debug)]
pub enum Cell {
    Quantum(Option<bool>),
    Discovered(Option<u8>),
    Concrete(bool),
}

impl Default for Cell {
    fn default() -> Self {
        Cell::Quantum(None)
    }
}

impl Cell {
    pub fn bomb_count(&self) -> RangeInclusive<u8> {
        match self {
            Cell::Quantum(None) => 0..=1,
            Cell::Quantum(Some(b)) | Cell::Concrete(b) => (*b as u8)..=(*b as u8),
            Cell::Discovered(_) => 0..=0,
        }
    }

    pub fn is_bomb(&self) -> bool {
        match self {
            Cell::Quantum(Some(b)) | Cell::Concrete(b) => *b,
            Cell::Quantum(None) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Board(Array2<Cell>);

impl Deref for Board {
    type Target = Array2<Cell>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Board {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Board {
    pub fn new(w: usize, h: usize) -> Self {
        Self(Array2::default((w, h)))
    }
    pub fn points(&self) -> impl Iterator<Item = (usize, usize)> {
        let (width, height) = self.dim();
        (0..width).cartesian_product(0..height)
    }
    pub fn neighbors(&self, x: usize, y: usize) -> impl Iterator<Item = (usize, usize, &Cell)> {
        (-1..=1isize)
            .cartesian_product(-1..=1isize)
            .filter(|p| *p != (0, 0))
            .filter_map(move |(dx, dy)| {
                x.checked_add_signed(dx)
                    .and_then(|x| Some((x, y.checked_add_signed(dy)?)))
            })
            .filter_map(|(x, y)| Some((x, y, self.get((x, y))?)))
    }
    /// Check whether assigning a particular value to a cell would violate any existing discovered cells
    pub fn assignment_is_legal(&self, x: usize, y: usize, value: bool) -> bool {
        let new_value = value as u8;
        let current_value = self[(x, y)].bomb_count();
        self.neighbors(x, y)
            .filter_map(|(x, y, c)| {
                if let Cell::Discovered(Some(n)) = c {
                    Some((x, y, *n))
                } else {
                    None
                }
            })
            .all(|(x, y, wants_bombs)| {
                let range = self.count_neighboring_bombs(x, y);
                let range = (range.start() - current_value.start() + new_value)
                    ..=(range.end() - current_value.end() + new_value);
                range.contains(&wants_bombs)
            })
    }
    /// Count the number of bombs neighboring a cell
    pub fn count_neighboring_bombs(&self, x: usize, y: usize) -> RangeInclusive<u8> {
        self.neighbors(x, y)
            .map(|(_, _, c)| c.bomb_count())
            .fold(0..=0, |acc, el| {
                (acc.start() + el.start())..=(acc.end() + el.end())
            })
    }
    /// Clear a cell, returning false if this cell was a bomb
    pub fn clear_cell(&mut self, x: usize, y: usize) -> bool {
        let c = self[(x, y)];

        match c {
            Cell::Quantum(Some(false)) | Cell::Concrete(false) => {
                self[(x, y)] = Cell::Discovered(None);
                true
            }
            Cell::Discovered(_) => true,
            _ => false,
        }
    }
    /// Find the values for all discovered cells
    pub fn find_discovered_counts(&self) -> Vec<((usize, usize), u8)> {
        let (width, height) = self.dim();
        (0..width)
            .cartesian_product(0..height)
            .filter_map(|(x, y)| {
                if !matches!(self[(x, y)], Cell::Discovered(None)) {
                    return None;
                }
                let range = self.count_neighboring_bombs(x, y);
                debug_assert_eq!(range.start(), range.end());
                Some(((x, y), *range.start()))
            })
            .collect()
    }
    /// Fill in discovered cells with their counts
    pub fn fill_discovered(&mut self) {
        let (width, height) = self.dim();
        (0..width)
            .cartesian_product(0..height)
            .filter_map(|(x, y)| {
                if !matches!(self[(x, y)], Cell::Discovered(_)) {
                    return None;
                }
                let range = self.count_neighboring_bombs(x, y);
                debug_assert_eq!(range.start(), range.end());
                Some(((x, y), *range.start()))
            })
            .collect_vec()
            .into_iter()
            .for_each(|(c, v)| self[c] = Cell::Discovered(Some(v)));
    }
    /// Collapse all quantum cells
    pub fn collapse(&mut self, mut max_bombs: usize) {
        eprintln!("Collapsing...");
        let (width, height) = self.dim();
        let mut validity_board = self.clone();
        validity_board.iter_mut().for_each(|c| {
            if matches!(c, Cell::Quantum(Some(_))) {
                *c = Cell::Quantum(None)
            }
        });
        let quantum_cells = (0..width)
            .cartesian_product(0..height)
            .filter(|(x, y)| match self[(*x, *y)] {
                Cell::Quantum(Some(b)) => {
                    if validity_board.assignment_is_legal(*x, *y, !b) {
                        true
                    } else {
                        max_bombs -= 1;
                        false
                    }
                }
                Cell::Quantum(_) => true,
                _ => false,
            })
            .filter(|(x, y)| {
                self.neighbors(*x, *y)
                    .any(|(_, _, n)| matches!(n, Cell::Discovered(_)))
            })
            .collect_vec();
        quantum_cells
            .iter()
            .for_each(|c| self[*c] = Cell::Quantum(None));
        eprintln!("{} quantum cells", quantum_cells.len());
        let mut states = self
            .collapse_inner(Arc::new(Cons::Empty), 0, &quantum_cells, max_bombs)
            .into_iter()
            .flatten()
            .map(|s| {
                let mut s = &s;
                std::iter::from_fn(move || {
                    if let Cons::Cell(b, next) = &**s {
                        s = next;
                        Some(*b)
                    } else {
                        None
                    }
                })
                .collect_vec()
                .into_iter()
                .rev()
                .collect_vec()
            })
            .collect_vec();
        eprintln!("{} possible states", states.len());
        if !states.is_empty() {
            let mut rng = rng();
            let began = Instant::now();
            let state_counts = (&mut rng)
                .sample_iter(Choose::new(states.as_slice()).unwrap())
                .take(states.len())
                .take_while(|_| began.elapsed() < Duration::from_secs(5))
                .map(|s| {
                    s.iter()
                        .zip(&quantum_cells)
                        .map(|(b, (x, y))| ((*x, *y), b))
                        .for_each(|(c, b)| self[c] = Cell::Quantum(Some(*b)));
                    (self.find_discovered_counts(), s)
                })
                .fold(HashMap::new(), |mut acc, (numbers, quanta)| {
                    acc.entry(numbers).or_insert((0usize, quanta)).0 += 1;
                    acc
                });
            eprintln!("{} unique sets of numbers", state_counts.len());
            if let Some((best_state, (_, quanta))) =
                state_counts.iter().max_by_key(|(_, count)| **count)
            {
                // best_state
                //     .iter()
                //     .for_each(|(c, v)| self[*c] = Cell::Discovered(Some(*v)));
                quantum_cells
                    .iter()
                    .zip(quanta.iter())
                    .for_each(|(c, v)| self[*c] = Cell::Quantum(Some(*v)));
            };
        }
    }
    fn collapse_inner<'a, 'b: 'a>(
        &mut self,
        list: Arc<Cons<bool>>,
        depth: usize,
        cells: &[(usize, usize)],
        max_bombs: usize,
    ) -> Option<Box<dyn Iterator<Item = Arc<Cons<bool>>> + Send + 'b>> {
        let [(x, y), rest @ ..] = cells else {
            if matches!(*list, Cons::Cell(_, _)) {
                return Some(Box::new(std::iter::once(list)));
            } else {
                return None;
            }
        };
        let left_is_legal = max_bombs > 0 && self.assignment_is_legal(*x, *y, true);
        let right_is_legal = self.assignment_is_legal(*x, *y, false);
        let depth_increase = (left_is_legal && right_is_legal) as usize;
        let left = {
            let list = list.clone();
            let mut board = self.clone();
            move || {
                if left_is_legal {
                    board[(*x, *y)] = Cell::Quantum(Some(true));
                    board
                        .collapse_inner(
                            Arc::new(Cons::Cell(true, list)),
                            depth + depth_increase,
                            rest,
                            max_bombs - 1,
                        )
                        .map(|i| {
                            Box::new(i) as Box<dyn Iterator<Item = Arc<Cons<bool>>> + Send + 'b>
                        })
                } else {
                    None
                }
            }
        };
        let right = {
            let list = list.clone();
            let mut board = self.clone();
            move || {
                if right_is_legal {
                    board[(*x, *y)] = Cell::Quantum(Some(false));
                    board
                        .collapse_inner(
                            Arc::new(Cons::Cell(false, list)),
                            depth + depth_increase,
                            rest,
                            max_bombs,
                        )
                        .map(|i| {
                            Box::new(i) as Box<dyn Iterator<Item = Arc<Cons<bool>>> + Send + 'b>
                        })
                } else {
                    None
                }
            }
        };
        let (left, right) = if depth < 5 {
            rayon::join(left, right)
        } else {
            (left(), right())
        };

        match (left, right) {
            (Some(left), Some(right)) => Some(Box::new(left.chain(right))),
            (Some(one), None) | (None, Some(one)) => Some(one),
            (None, None) => None,
        }
    }
}

#[derive(Debug)]
enum Cons<T> {
    Empty,
    Cell(T, Arc<Self>),
}

// pub enum Board {
//     Quad([[Arc<Board>; 2]; 2]),
//     Concrete(Array2<Cell>),
// }

// impl Board {
//     pub fn size(&self) -> (usize, usize) {
//         match self {
//             Board::Quad([[tl, tr], [bl, br]]) => {
//                 let tl = tl.size();
//                 let tr = tr.size();
//                 let bl = bl.size();
//                 let br = br.size();
//                 let top_width = tl.0 + tr.0;
//                 let bottom_width = bl.0 + br.0;
//                 debug_assert_eq!(top_width, bottom_width);
//                 let left_height = tl.1 + bl.1;
//                 let right_height = tr.1 + br.1;
//                 debug_assert_eq!(left_height, right_height);
//                 (top_width, left_height)
//             }
//             Board::Concrete(array) => array.dim(),
//         }
//     }

//     pub fn call_descend(&self, )

//     pub fn assignment_is_legal(&self, x: usize, y: usize, value: bool) -> bool {
//         todo!()
//     }

//     pub fn collapse(self: Arc<Self>, maximize: impl Fn(&Self) -> f64) -> Arc<Self> {
//         match *self {
//             Board::Quad(_) => todo!(),
//             Board::Concrete(contents) => {}
//         }
//     }
// }

// impl Index<(usize, usize)> for Board {
//     type Output = Cell;

//     fn index(&self, index: (usize, usize)) -> &Self::Output {
//         match self {
//             Board::Quad(_) => todo!(),
//             Board::Concrete(array) => &array[index],
//         }
//     }
// }

// impl IndexMut<(usize, usize)> for Board {
//     type Output = Cell;

//     fn index_mut(&mut self, index: (usize, usize)) -> &mut Self::Output {
//         todo!()
//     }
// }
